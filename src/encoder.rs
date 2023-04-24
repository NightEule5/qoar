// Copyright 2023 Strixpyrr
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod slice_scaler;
use slice_scaler::{LinearScaler, VectorScaler};

use std::cmp::min;
use std::result;
use std::error::Error;
use amplify_derive::Display;
use crate::{DescriptorError, FRAME_LEN, MAGIC, PcmBuffer, PcmSink, PcmSource, QoaLmsState, SLICE_LEN, StreamDescriptor};
use crate::io::{SinkStream, WriteError};
use EncodeError::*;
use WriteKind::*;

type Result<T = ()> = result::Result<T, EncodeError>;

#[derive(Debug, Display)]
pub enum EncodeError {
	#[display("invalid stream descriptor ({0}); use streaming mode if unknown")]
	InvalidDescriptor(DescriptorError),
	#[display("stream descriptor cannot be set in a fixed encoder")]
	InvalidDescriptorChange,
	#[display("could not read samples")]
	SampleRead(Box<dyn Error>),
	#[display("could not write {0} ({1})")]
	Write(WriteKind, WriteError),
	#[display("could not flush the sink ({0})")]
	Flush(WriteError),
	#[display("closed")]
	Closed,
}

#[derive(Clone, Debug, Display)]
pub enum WriteKind {
	#[display("file header")]
	FileHeader,
	#[display("frame header")]
	FrameHeader,
	#[display("lms {0}")]
	LmsState(&'static str),
	#[display("slice data on channel {0}")]
	SliceData(u8),
}

impl Error for EncodeError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			SampleRead(err) => Some(err.as_ref()),
			Write(_, err) => Some(err),
			_ => None
		}
	}
}

pub(crate) struct Frame {
	slice_width: usize,
	slice_count: u16,
	slice_index: u16,
	buffer: PcmBuffer,
}

impl Frame {
	fn new(channel_count: usize) -> Self {
		Self {
			slice_width: SLICE_LEN * channel_count,
			slice_count: 0,
			slice_index: 0,
			buffer: PcmBuffer::default(),
		}
	}

	/// Returns `true` if the frame header has been written.
	fn start(&mut self, samples: usize, channels: usize) -> bool {
		if self.complete() {
			self.buffer.clear();
			let samples = min(FRAME_LEN, samples);
			self.slice_width = SLICE_LEN * channels;
			self.slice_count = ((samples + SLICE_LEN - 1) / SLICE_LEN) as u16;
			self.slice_index = 0;
			self.buffer.set_descriptor(SLICE_LEN as u32, channels).unwrap();
			true
		} else {
			false
		}
	}

	/// Returns `true` if all slices have been written.
	fn complete(&self) -> bool { self.slice_index >= self.slice_count }

	fn reset(&mut self) {
		self.slice_count = 0;
		self.slice_index = 0;
	}
}

pub trait SliceScaler: slice_scaler::SliceScaler { }

impl<S: slice_scaler::SliceScaler> SliceScaler for S { }

#[cfg(feature = "simd")]
pub type SimdEncoder<S> = Encoder<S, VectorScaler>;

pub struct Encoder<S: SinkStream, Sc: SliceScaler = LinearScaler> {
	desc: StreamDescriptor,
	sink: Option<S>,
	has_header: bool,
	lms_states: Vec<QoaLmsState>,
	frame: Frame,
	_scaler: Sc,
}

impl<S: SinkStream> Encoder<S> {
	pub fn new_fixed(sample_count: usize, sample_rate: u32, channel_count: usize, sink: S) -> Result<Self> {
		Self::_new_fixed(sample_count, sample_rate, channel_count, sink, LinearScaler)
	}

	pub fn new_streaming(sink: S) -> Self {
		Self::_new_streaming(sink, LinearScaler)
	}
}

#[cfg(feature = "simd")]
impl<S: SinkStream> SimdEncoder<S> {
	pub fn new_fixed_simd(sample_count: usize, sample_rate: u32, channel_count: usize, sink: S) -> Result<Self> {
		Self::_new_fixed(
			sample_count,
			sample_rate,
			channel_count,
			sink,
			VectorScaler
		)
	}

	pub fn new_streaming_simd(sink: S) -> Self {
		Self::_new_streaming(sink, VectorScaler)
	}
}

impl<S: SinkStream, Sc: SliceScaler> Encoder<S, Sc> {
	fn _new_fixed(sample_count: usize, sample_rate: u32, channel_count: usize, sink: S, scaler: Sc) -> Result<Self> {
		Ok(Self {
			desc: StreamDescriptor::new(
				Some(sample_count),
				Some(sample_rate),
				Some(channel_count)
			).map_err(InvalidDescriptor)?,
			sink: Some(sink),
			has_header: false,
			lms_states: vec![QoaLmsState::default(); channel_count as usize],
			frame: Frame::new(channel_count),
			_scaler: scaler,
		})
	}

	fn _new_streaming(sink: S, scaler: Sc) -> Self {
		Self {
			desc: StreamDescriptor::default(),
			sink: Some(sink),
			has_header: false,
			lms_states: Vec::new(),
			frame: Frame::new(0),
			_scaler: scaler,
		}
	}

	/// Encodes samples from a [`Vec`].
	pub fn encode_vec(&mut self, source: &mut Vec<i16>, mut desc: StreamDescriptor) -> Result {
		let Self { desc: this_desc, has_header, lms_states, frame, .. } = self;
		desc.infer_from_vec(source, this_desc);

		if this_desc.is_streaming() {
			this_desc.sample_rate   = desc.sample_rate;
			this_desc.channel_count = desc.channel_count;
		} else {
			if desc.sample_rate   != this_desc.sample_rate ||
				desc.channel_count != this_desc.channel_count {
				return Err(InvalidDescriptorChange)
			}
		}

		let (samples, rate, channels) = desc.unwrap_all();

		if samples == 0 || rate == 0 || channels == 0 {
			return Ok(())
		}


		let mut samples = samples as usize;

		{
			let sink = self.sink.as_mut().ok_or(Closed)?;

			if *has_header {
				sink.enc_file_header(this_desc.sample_count.unwrap_or_default())?;
				*has_header = true;
			}

			lms_states.resize(channels as usize, QoaLmsState::default());

			while let n @ 1.. = sink.enc_frame::<Sc>(source, samples, channels, rate, lms_states, frame)? {
				source.truncate(source.len().saturating_sub(n * channels as usize));
				samples = samples.saturating_sub(n);
			}

			sink.flush().map_err(Flush)?
		}

		self.set_sample_count(samples as u32);
		Ok(())
	}

	/// Encodes samples from a [`Pcm16Source`].
	pub fn encode(&mut self, source: &mut impl PcmSource) -> Result {
		let mut desc = source.descriptor();
		let Self { desc: this_desc, has_header, lms_states, frame, .. } = self;
		desc.infer(this_desc);

		if !this_desc.is_streaming() {
			if desc.sample_rate   != this_desc.sample_rate ||
				desc.channel_count != this_desc.channel_count {
				return Err(InvalidDescriptorChange)
			}
		}

		let (samples, rate, channels) = desc.unwrap_all();

		if samples == 0 || rate == 0 || channels == 0 {
			return Ok(())
		}

		let mut samples = samples as usize;

		{
			let sink = self.sink.as_mut().ok_or(Closed)?;

			if *has_header {
				sink.enc_file_header(this_desc.sample_count.unwrap_or_default())?;
				*has_header = true;
			}

			lms_states.resize(channels as usize, QoaLmsState::default());

			frame.start(samples, channels as usize);

			while !source.read(&mut frame.buffer, frame.slice_width)
						 .map_err(|err| SampleRead(err.into()))? > 0 {
				let n = sink.enc_frame::<Sc>(&[], samples, channels, rate, lms_states, frame)?;
				samples = samples.saturating_sub(n);

				if n == 0 {
					break
				}
			}

			sink.flush().map_err(Flush)?;
		}

		self.set_sample_count(samples as u32);
		Ok(())
	}

	/// Flushes buffered samples to the inner sink.
	pub fn flush(&mut self) -> Result<()> {
		self.encode_vec(&mut Vec::new(), StreamDescriptor::default())
	}

	/// Closes the encoder, returning the inner sink if not already closed.
	pub fn close(&mut self) -> Option<Result<S>> {
		match self.flush() {
			Err(Closed) => return None,
			Err(err) => return Some(Err(err)),
			_ => { }
		}

		self.sink.take().map(Ok)
	}

	/// Sets the sample count after encoding, if in fixed mode.
	fn set_sample_count(&mut self, n: u32) {
		if let Some(ref mut samples) = self.desc.sample_count {
			*samples = n as usize;
		}
	}
}

impl<S: SinkStream, Sc: SliceScaler> Drop for Encoder<S, Sc> {
	/// Closes the encoder.
	fn drop(&mut self) { let _ = self.close(); }
}

pub(crate) trait QoaSink: SinkStream {
	fn enc_file_header(&mut self, sample_count: usize) -> Result {
		self.write_long((MAGIC as u64) << 32 | sample_count as u64)
			.map_err(|err| Write(FileHeader, err))
	}

	fn enc_frame_header(
		&mut self,
		channel_count: usize,
		sample_rate: u32,
		sample_count: u16,
		size: u16
	) -> Result {
		let mut value = channel_count as u64;
		value <<= 24;
		value |= sample_rate as u64;
		value <<= 16;
		value |= sample_count as u64;
		value <<= 16;
		value |= size as u64;

		self.write_long(value)
			.map_err(|err| Write(FrameHeader, err))
	}

	fn enc_lms_state(&mut self, value: &QoaLmsState) -> Result {
		let QoaLmsState { history, weights } = value;

		fn pack(acc: u64, cur: &i32) -> u64 {
			(acc << 16) | *cur as i16 as u16 as u64
		}

		let history = history.iter().fold(0, pack);
		let weights = weights.iter().fold(0, pack);
		self.write_long(history).map_err(|err| Write(LmsState("history"), err))?;
		self.write_long(weights).map_err(|err| Write(LmsState("weights"), err))
	}

	fn enc_slice<Scaler: SliceScaler>(
		&mut self,
		samples: &[i16],
		channel_count: usize,
		lms: &mut [QoaLmsState]
	) -> Result {
		for chn in 0..channel_count {
			self.write_long(Scaler::scale(samples, &mut lms[chn], chn, channel_count))
				.map_err(|err|
					Write(SliceData(chn as u8), err)
				)?;
		}

		Ok(())
	}

	fn enc_frame<Scaler: SliceScaler>(
		&mut self,
		sample_buf: &[i16],
		sample_cnt: usize,
		channels: usize,
		rate: u32,
		lms: &mut [QoaLmsState],
		frame: &mut Frame,
	) -> Result<usize> {
		if sample_buf.is_empty() && frame.buffer.is_empty() {
			return Ok(0)
		}

		let mut consumed = 0;
		let mut off = 0;
		let mut len = sample_buf.len();

		if frame.start(sample_cnt, channels as usize) {
			let size = 24 * channels as u16 + 8 * frame.slice_count as u16 * channels as u16;
			self.enc_frame_header(channels, rate, sample_cnt as u16, size)?;

			for lms in lms.iter() { self.enc_lms_state(lms)? }
		}

		{
			let Frame { slice_width, slice_count, slice_index, buffer, .. } = frame;

			len = min(len, *slice_width * *slice_count as usize);

			if !buffer.is_empty() || len <= *slice_width {
				off = min((*slice_width).saturating_sub(buffer.len()), len);
				consumed = SLICE_LEN;
				buffer.write_interleaved(&sample_buf[..off]).unwrap();

				if buffer.len() <= SLICE_LEN {
					//self.enc_slice(buffer., channels, lms)?;
					*slice_index += 1;
					buffer.clear();
				}
			}
		}

		let slices = sample_buf[off..len].chunks_exact(frame.slice_width);
		let excess = slices.remainder();
		for slice in slices {
			if frame.complete() { break }

			self.enc_slice::<Scaler>(slice, channels as usize, lms)?;
			frame.slice_index += 1;
			consumed += SLICE_LEN;
		}

		frame.buffer.write_interleaved(excess).unwrap();

		if frame.complete() {
			frame.reset();
		}

		Ok(consumed)
	}
}

impl<S: SinkStream> QoaSink for S { }
