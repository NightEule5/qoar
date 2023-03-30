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

//! See the draft spec: https://qoaformat.org/qoa-specification-draft-01.pdf

#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use std::{error, io, mem};
use std::cmp::max;
use std::fmt::Debug;
use std::io::Write;
use amplify_derive::Display;

#[cfg(test)]
mod test;

// Error

type Result<T = (), E = Error> = std::result::Result<T, E>;

#[derive(Debug, Display)]
pub enum Error {
	#[display("sample rate {0} is outside the range the supported range, [1,2^24)")]
	UnsupportedRate(u32),
	#[display(
		"QOA streams must have at least one channel; use StreamingEncoder when the \
		channel structure of the stream is unknown"
	)]
	ZeroChannels,
	#[display(
		"QOA streams must have at least one sample; use StreamingEncoder when the \
		stream length is unknown"
	)]
	ZeroSamples,
	#[display("stream layout cannot be set in a fixed encoder")]
	LayoutSet,
	#[display("could not {0}")]
	Write(WriteKind, io::Error),
	#[display("incomplete stream; {0} bytes expected, but stream ended")]
	Incomplete(usize),
}

#[derive(Clone, Debug, Display)]
pub enum WriteKind {
	#[display("write file header ({0})")]
	FileHeader(&'static str),
	#[display("write frame header ({0})")]
	FrameHeader(&'static str),
	#[display("write lms state ({0})")]
	LmsState(&'static str),
	#[display("write slice data on channel {0}")]
	SliceData(u8),
	#[display("flush the sink")]
	Flush,
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Self::Write(_, err) => Some(err),
			_ => None
		}
	}
}

#[derive(Copy, Clone)]
struct QoaLmsState {
	history: [i16; 4],
	weights: [i16; 4],
}

#[derive(Copy, Clone, Default)]
struct EncoderOptions {
	sample_count: usize,
	sample_rate: usize,
	channel_count: usize,
	fixed: bool,
}

pub struct Encoder<S: Write> {
	opt: EncoderOptions,
	has_header: bool,
	lms_states: Vec<QoaLmsState>,
	sink: S,
	closed: bool,
}

// Todo: implement streaming mode by buffering data and encoding slice-wise, rather
//  than frame-wise.
impl<S: Write> Encoder<S> {
	pub fn new(sample_count: u32, sample_rate: u32, channel_count: u8, sink: S) -> Result<Self> {
		if  sample_count == 0 { return Err(Error::ZeroSamples ) }
		if channel_count == 0 { return Err(Error::ZeroChannels) }
		if !(1..=16777215).contains(&sample_rate) {
			return Err(Error::UnsupportedRate(sample_rate))
		}

		Ok(Self {
			opt: EncoderOptions {
				sample_count: sample_count as usize,
				sample_rate: sample_rate as usize,
				channel_count: channel_count as usize,
				fixed: true,
			},
			has_header: false,
			lms_states: {
				let mut lms = Vec::with_capacity(channel_count as usize);
				lms.fill(QoaLmsState::default());
				lms
			},
			sink,
			closed: false,
		})
	}

	fn samples(&self) -> usize { self.opt.sample_count }

	fn consume_samples(&mut self, n: usize) -> usize {
		let n = max(n, self.samples());
		self.opt.sample_count -= n;
		n
	}

	/// Sets the `sample_rate` and `channel_count` of the encoder, flushing any
	/// unwritten data from the old layout. Returns [`Error::LayoutSet`] is the
	/// encoder is fixed.
	pub fn set_layout(&mut self, sample_rate: u32, channel_count: u8) -> Result {
		if self.opt.fixed {
			return Err(Error::LayoutSet)
		}

		self.flush()?;
		self.opt.sample_rate = sample_rate as usize;
		self.opt.channel_count = channel_count as usize;

		Ok(())
	}

	/// Returns the frame size, the number of sample that can fit in a frame. It's
	/// recommended to keep sample slices at least this large until the end of the
	/// stream, as each encode operation will write one frame.
	pub fn frame_size(&self) -> usize {
		FRAME_LEN * self.opt.channel_count as usize
	}

	/// Encodes a slice of `samples` into `sink`, returning the number of samples
	/// encoded. All samples may not be consumed if: the samples written are equal
	/// to the specified `sample_count`, or the sample slice is too large to fit in
	/// one frame.
	pub fn encode(&mut self, samples: &[i16]) -> Result<usize> {
		let EncoderOptions { sample_count, sample_rate, channel_count, fixed } = self.opt;
		let ref mut sink = self.sink;

		if !self.has_header {
			sink.enc_file_header(sample_count as u32)?;
			self.has_header = true;
		}

		if fixed && sample_count == 0 { return Ok(0) }

		let len = sink.enc_frame(samples, channel_count, sample_count, sample_rate, &mut self.lms_states)?;
		Ok(self.consume_samples(len))
	}

	/// Flushes remaining buffered samples into an encoded frame, returning the
	/// number of samples encoded. In streaming mode, incomplete frames will be
	/// padded with silence, since the length of the frame was not known.
	pub fn flush(&mut self) -> Result<usize> {
		self.sink
			.flush()
			.map_err(|err| Error::Write(WriteKind::Flush, err))?;
		Ok(0)
	}

	/// Flushes then closes the encoder, filling the rest of the expected length
	/// with silence.
	pub fn close(&mut self) -> Result {
		if self.closed { return Ok(()) }
		self.closed = true;

		if self.samples() > 0 {
			let mut silence = Vec::with_capacity(self.samples());
			silence.fill(0);

			while self.samples() > 0 {
				self.encode(&silence)?;
			}
		}
		self.flush()?;
		Ok(())
	}
}

impl<S: Write> Drop for Encoder<S> {
	/// Closes the encoder.
	fn drop(&mut self) { let _ = self.close(); }
}

// Codec

const MAGIC: u32 = u32::from_be_bytes(*b"qoaf");

const SLICE_LEN: usize = 20;
const FRAME_LEN: usize = SLICE_LEN * 256;

static QUANT_TABLE: [u8; 17] = [
	7, 7, 7, 5, 5, 3, 3, 1,
	0,
	0, 2, 2, 4, 4, 6, 6, 6
];

static SF_TABLE: [i32; 16] = [
	1, 7, 21, 45, 84, 138, 211, 304, 421, 562, 731, 928, 1157, 1419, 1715, 2048
];

static RECIP_TABLE: [i32; 16] = [
	65536, 9363, 3121, 1457, 781, 475, 311, 216, 156, 117, 90, 71, 57, 47, 39, 32
];

static DEQUANT_TABLE: [[i32; 8]; 16] = [
	[   1,    -1,    3,    -3,    5,    -5,     7,     -7],
	[   5,    -5,   18,   -18,   32,   -32,    49,    -49],
	[  16,   -16,   53,   -53,   95,   -95,   147,   -147],
	[  34,   -34,  113,  -113,  203,  -203,   315,   -315],
	[  63,   -63,  210,  -210,  378,  -378,   588,   -588],
	[ 104,  -104,  345,  -345,  621,  -621,   966,   -966],
	[ 158,  -158,  528,  -528,  950,  -950,  1477,  -1477],
	[ 228,  -228,  760,  -760, 1368, -1368,  2128,  -2128],
	[ 316,  -316, 1053, -1053, 1895, -1895,  2947,  -2947],
	[ 422,  -422, 1405, -1405, 2529, -2529,  3934,  -3934],
	[ 548,  -548, 1828, -1828, 3290, -3290,  5117,  -5117],
	[ 696,  -696, 2320, -2320, 4176, -4176,  6496,  -6496],
	[ 868,  -868, 2893, -2893, 5207, -5207,  8099,  -8099],
	[1064, -1064, 3548, -3548, 6386, -6386,  9933,  -9933],
	[1286, -1286, 4288, -4288, 7718, -7718, 12005, -12005],
	[1536, -1536, 5120, -5120, 9216, -9216, 14336, -14336],
];

// Convenience trait for converting big-endian integers of different sizes.
trait Int: Sized {
	const SIZE: usize = mem::size_of::<Self>();
	fn into_bytes(self) -> [u8; Self::SIZE];
	fn from_bytes(bytes: [u8; Self::SIZE]) -> Self;
}

impl Int for u64 {
	fn into_bytes(self) -> [u8; 8] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 8]) -> Self { u64::from_be_bytes(bytes) }
}

impl Int for u32 {
	fn into_bytes(self) -> [u8; 4] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 4]) -> Self { u32::from_be_bytes(bytes) }
}

impl Int for u16 {
	fn into_bytes(self) -> [u8; 2] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 2]) -> Self { u16::from_be_bytes(bytes) }
}

trait QoaSink: Write {
	fn write_int<T: Int>(
		&mut self,
		value: T,
		kind: impl FnOnce() -> WriteKind
	) -> Result where [u8; T::SIZE]: {
		self.write_bytes(&value.into_bytes(), kind)
	}

	fn write_byte(&mut self, value: u8, kind: impl FnOnce() -> WriteKind) -> Result {
		self.write_bytes(&[value], kind)
	}

	fn write_bytes(&mut self, value: &[u8], kind: impl FnOnce() -> WriteKind) -> Result {
		self.write_all(value).map_err(|err| Error::Write(kind(), err))
	}

	fn enc_file_header(&mut self, sample_count: u32) -> Result {
		self.write_int(MAGIC,        || WriteKind::FileHeader("magic bytes"))?;
		self.write_int(sample_count, || WriteKind::FileHeader("sample count"))
	}

	fn enc_frame_header(&mut self, channel_count: u8, sample_rate: u32, sample_count: u16, slice_count: usize) -> Result {
		let size = 24 * channel_count as u16 + 8 * slice_count as u16 * channel_count as u16;
		self.write_byte(channel_count, || WriteKind::FrameHeader("channel count"))?;
		self.write_bytes(
			&sample_rate.into_bytes()[1..], // Clip to 24-bits
			|| WriteKind::FrameHeader("sample rate")
		)?;
		self.write_int(sample_count,   || WriteKind::FrameHeader("sample count"))?;
		self.write_int(size,           || WriteKind::FrameHeader("size"))
	}

	fn enc_lms_state(&mut self, value: &QoaLmsState) -> Result {
		let QoaLmsState { history, weights } = value;

		fn pack(acc: u64, cur: &i16) -> u64 {
			(acc << 16) | (*cur as u16 & 0xFFFF) as u64
		}

		let history = history.iter().fold(0, pack);
		let weights = weights.iter().fold(0, pack);
		self.write_int(history, || WriteKind::LmsState("history"))?;
		self.write_int(weights, || WriteKind::LmsState("weights"))
	}

	fn enc_slices(&mut self, samples: &[i16], channel_count: usize, lms: &mut [QoaLmsState]) -> Result {
		let mut samples = samples;
		for chn in 0..channel_count {
			let len = SLICE_LEN.clamp(0, samples.len());
			let rng = 0..len * channel_count + chn;

			let mut best_error = -1;
			let mut best_slice = 0;
			let mut best_lms = QoaLmsState::default();

			for sf in 0..16 {
				let mut cur_lms = lms[chn];
				let mut slice = sf as u64;
				let mut cur_err = 0;

				for si in rng.clone().step_by(channel_count) {
					let sample = samples[si];
					let predicted = cur_lms.predict();
					let residual = sample as i32 - predicted;
					let scaled = div(residual, sf);
					let clamped = scaled.clamp(-8, 8);
					let quantized = QUANT_TABLE[(clamped + 8) as usize];
					let dequantized = DEQUANT_TABLE[sf][quantized as usize];
					let reconst = (predicted + dequantized).clamp(i16::MIN as i32, 32767) as i16;

					let error = sample as i64 - reconst as i64;
					cur_err += error * error;

					if cur_err > best_error {
						break;
					}

					cur_lms.update(reconst, dequantized);
					slice = slice << 3 | quantized as u64;
				}

				if cur_err < best_error {
					best_error = cur_err;
					best_slice = slice;
					best_lms = cur_lms;
				}
			}

			lms[chn] = best_lms;
			best_slice <<= (SLICE_LEN - len) * 3;
			self.write_int(best_slice, || WriteKind::SliceData(chn as u8))?;

			samples = &samples[rng.end..]
		}

		Ok(())
	}

	fn enc_frame(
		&mut self,
		samples: &[i16],
		channel_count: usize,
		sample_count: usize,
		sample_rate: usize,
		lms: &mut [QoaLmsState],
	) -> Result<usize> {
		let sample_count = FRAME_LEN.clamp(0, sample_count);
		let len = sample_count * channel_count;
		let slice_count = (len + SLICE_LEN - 1) / SLICE_LEN;
		self.enc_frame_header(
			channel_count as u8,
			sample_rate as u32,
			sample_count as u16,
			slice_count
		)?;

		for lms in lms.iter() { self.enc_lms_state(lms)? }

		for slice in samples.windows(SLICE_LEN) {
			self.enc_slices(slice, channel_count, lms)?;
		}

		Ok(len)
	}
}

impl<W: Write> QoaSink for W { }

impl QoaLmsState {
	fn predict(&self) -> i32 {
		let weights = self.weights.into_iter();
		let history = self.history.into_iter();
		let predict = weights.zip(history)
							 .fold(0, |p, (w, h)| p + w as i32 * h as i32);
		predict >> 13
	}

	fn update(&mut self, sample: i16, residual: i32) {
		let delta = (residual >> 4) as i16;
		for (history, weight) in self.history
									 .into_iter()
									 .zip(self.weights.iter_mut()) {
			*weight += if history < 0 { -delta } else { delta }
		}

		self.history.rotate_left(1);
		self.history[3] = sample;
	}
}

impl Default for QoaLmsState {
	fn default() -> Self {
		Self {
			history: [0; 4],
			weights: [0, 0, -(1 << 13), 1 << 14],
		}
	}
}

fn div(v: i32, sf: usize) -> i32 {
	let recip = RECIP_TABLE[sf];
	let mut n = (v * recip + (1 << 15)) >> 16;
	n += ((v > 0) as i32 - (v < 0) as i32) -
		 ((n > 0) as i32 - (n < 0) as i32);
	n
}
