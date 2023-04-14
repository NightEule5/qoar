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

use std::{error, mem};
use std::cmp::min;
use amplify_derive::Display;
use crate::{FRAME_LEN, StreamDescriptor};
use crate::util::Then;

// Stream traits

/// A stream of PCM samples.
pub trait PcmStream {
	/// Returns the current channel count of the stream, or `0` if not known.
	fn channel_count(&self) -> usize;
	/// Returns the current sample rate of the stream, or `0` if not known.
	fn sample_rate(&self) -> u32;
}

#[derive(Debug, Display)]
pub enum Error {
	#[display("cannot set immutable stream descriptor")]
	DescriptorSet,
	#[display("attempted to write without setting descriptor")]
	UninitializedDescriptor,
	#[display("cannot read samples")]
	Read(Box<dyn error::Error>),
	#[display("cannot write samples")]
	Write(Box<dyn error::Error>),
	#[display("{0}")]
	Other(Box<dyn error::Error>),
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		match self {
			Self::Read (err) |
			Self::Write(err) |
			Self::Other(err) => Some(err.as_ref()),
			_ => None
		}
	}
}

/// A PCM16-LE source.
pub trait PcmSource: PcmStream {
	/// Reads a maximum of `sample_count` samples into the [`PcmSink`], returns the
	/// number of samples read.
	fn read(&mut self, buf: &mut impl PcmSink, sample_count: usize) -> Result<usize, Error>;

	fn read_all(mut self) -> Result<PcmBuffer, Error> where Self: Sized {
		let mut buf = PcmBuffer::default();
		let rate = self.sample_rate();
		let chan = self.channel_count();

		buf.set_descriptor(rate, chan)?;

		let mut cnt = self.sample_count();
		while self.read(&mut buf, cnt)? > 0 {
			cnt = self.sample_count();
		}

		Ok(buf)
	}

	/// Returns the number of samples per channel available, or `0` if not known.
	fn sample_count(&self) -> usize;

	/// Gets a [`StreamDescriptor`] instance describing the source.
	fn descriptor(&self) -> StreamDescriptor {
		let samples  = self.sample_count();
		let rate     = self.sample_rate().clamp(0, 16777216);
		let channels = self.channel_count();
		StreamDescriptor::new(
			(samples  > 0).then(|| samples ),
			(rate     > 0).then(|| rate    ),
			(channels > 0).then(|| channels),
		).unwrap_or_default()
	}
}

/// A PCM16-LE sink.
pub trait PcmSink: PcmStream {
	/// Writes samples from `buf` into `chn`, then returns the number of samples
	/// written. Care should be taken for channels to be written with same-length
	/// buffers and in sequence; there are no guarantees of behavior if not.
	fn write(&mut self, buf: &[i16], chn: usize) -> Result<usize, Error>;

	/// Writes interleaved samples from `buf`, then returns the number of slices
	/// written, or the total number of samples written if the channel count isn't
	/// known.
	fn write_interleaved(&mut self, buf: &[i16]) -> Result<usize, Error>;
	
	/// Writes a frame, returning the partially consumed frame if it could not be
	/// completely written.
	fn write_frame(&mut self, frame: PcmFrame) -> Result<Option<PcmFrame>, Error> {
		self.set_descriptor(frame.rate, frame.chan)?;
		self.write_interleaved(frame.data())?;
		Ok(if frame.is_empty() {
			None
		} else {
			Some(frame)
		})
	}

	/// Returns maximum number of samples per channel that can be written to this
	/// sink.
	fn sample_capacity(&self) -> usize;

	/// Sets the `sample_rate` and `channel_count`.
	fn set_descriptor(
		&mut self,
		sample_rate: u32,
		channel_count: usize
	) -> Result<(), Error>;

	/// Flushes buffered samples.
	fn flush(&mut self) -> Result<(), Error> { Ok(()) }
	/// Closes the sink.
	fn close(&mut self) -> Result<(), Error> { self.flush() }
}

// Buffer

#[derive(Clone, Debug, Eq)]
pub struct PcmFrame {
	data: Vec<i16>,
	len : usize,
	size: usize,
	rate: u32,
	chan: usize,
}

impl PcmFrame {
	/// Creates a new frame, using `sample_count` and `channel_count` to size it.
	///
	/// # Panics
	///
	/// Panics if `sample_rate` or `channel_count` are `0`.
	pub fn new(sample_count: usize, sample_rate: u32, channel_count: usize) -> Self {
		assert_ne!(sample_rate, 0, "sample rate must be known");
		assert_ne!(channel_count, 0, "channel count must be known");

		Self {
			data: Vec::with_capacity(sample_count * channel_count),
			len : 0,
			size: sample_count,
			rate: sample_rate,
			chan: channel_count,
		}
	}

	/// Trims and returns the internal sample buffer.
	pub fn unwrap(mut self) -> Vec<i16> {
		self.trim();
		self.data
	}

	/// Returns the number of samples per channel currently in the frame.
	pub fn len(&self) -> usize { self.len }

	pub fn is_empty(&self) -> bool { self.len == 0 }

	pub fn is_full(&self) -> bool { self.len >= self.size }

	/// Returns the sample rate.
	pub fn rate(&self) -> u32 { self.rate }

	/// Returns the channel count.
	pub fn channels(&self) -> usize { self.chan }

	/// Returns the current sample data.
	pub fn data(&self) -> &[i16] { &self.data }

	/// Grows the frame by `sample_count` samples.
	pub fn grow(&mut self, sample_count: usize) {
		self.size += sample_count;
		self.data.reserve(sample_count * self.chan);
	}

	/// Trims the frame such that its max size equals its length.
	pub fn trim(&mut self) {
		self.size = self.len();
		self.data.truncate(self.size * self.chan);
	}
}

impl PartialEq for PcmFrame {
	fn eq(&self, other: &Self) -> bool {
		self.data == other.data &&
		self.rate == other.rate &&
		self.chan == other.chan
	}
}

impl PcmStream for PcmFrame {
	fn channel_count(&self) -> usize { self.chan }

	fn sample_rate(&self) -> u32 { self.rate }
}

impl PcmSource for PcmFrame {
	fn read(&mut self, buf: &mut impl PcmSink, sample_count: usize) -> Result<usize, Error> {
		let samples = min(sample_count, self.len * self.chan);
		let read = buf.write_interleaved(&self.data()[..samples])?;
		self.len = self.len.saturating_sub(read);
		self.data.truncate(self.len * self.chan);
		Ok(read)
	}

	fn sample_count(&self) -> usize { self.len }
}

impl PcmSink for PcmFrame {
	fn write(&mut self, data: &[i16], chn: usize) -> Result<usize, Error> {
		assert!(chn < self.chan, "channel index out of bounds");

		let len = self.len();
		let samples = min(data.len(), self.size - len);

		if samples == 0 { return Ok(0) }

		if chn == 0 {
			self.data.resize(len + samples * self.chan, 0);
		} else if chn == self.chan - 1 {
			self.len += samples;
		}

		let off = len * self.chan + chn;
		for i in 0..samples {
			self.data[off + i] = data[i]
		}

		Ok(samples)
	}

	fn write_interleaved(&mut self, data: &[i16]) -> Result<usize, Error> {
		self.len = self.data.len();
		let samples = min(data.len() / self.chan, self.sample_capacity());
		self.len += samples;

		let total = samples * self.chan;
		self.data.extend_from_slice(&data[..total]);
		Ok(total)
	}

	fn write_frame(&mut self, mut frame: Self) -> Result<Option<Self>, Error> {
		self.set_descriptor(frame.rate, frame.chan)?;
		frame.read(self, frame.sample_count())?;
		Ok(frame.is_empty().then(|| frame))
	}
	
	fn sample_capacity(&self) -> usize { self.size - self.len }

	fn set_descriptor(&mut self, sample_rate: u32, channel_count: usize) -> Result<(), Error> {
		(sample_rate != self.rate || channel_count != self.chan)
			.then_err(Error::DescriptorSet)
	}
}

/// A buffer of interleaved PCM samples.
#[derive(Clone, Debug)]
pub struct PcmBuffer {
	buf: Vec<PcmFrame>,
	frame_size: usize,
}

impl PcmBuffer {
	pub fn new(frame_size: usize) -> Self {
		assert!(frame_size > 0, "frame size must be non-zero");

		Self {
			buf: Vec::default(),
			frame_size,
		}
	}

	pub fn len(&self) -> usize { self.buf.iter().map(PcmFrame::len).sum() }

	pub fn is_empty(&self) -> bool { self.len() == 0 }

	pub fn clear(&mut self) {
		self.buf.clear();
	}

	/// Returns the underlying frame buffer.
	pub fn unwrap(self) -> Vec<PcmFrame> { self.buf }

	/// Copies sample data into a byte vector. Sample rate and channel information
	/// is lost.
	pub fn encode(&self) -> Vec<u8> {
		let len = self.len();
		let mut buf = Vec::with_capacity(len * mem::size_of::<i16>());

		for frame in &self.buf {
			for sample in &frame.data {
				buf.extend_from_slice(&sample.to_le_bytes());
			}
		}

		buf
	}

	fn new_frame(&mut self, rate: u32, channels: usize) {
		self.buf.push(PcmFrame::new(self.frame_size, rate, channels))
	}

	fn frame(&self) -> Option<&PcmFrame> { self.buf.last() }

	fn pop_frame(&mut self, rate: u32, channels: usize) -> PcmFrame {
		self.set_descriptor(rate, channels).unwrap();
		self.buf.pop().unwrap()
	}

	fn descriptor(&self) -> Result<(u32, usize), Error> {
		self.frame()
			.map(|f| (f.rate, f.chan))
			.ok_or(Error::UninitializedDescriptor)
	}

	fn write_with<F: Fn(&mut PcmFrame, &[i16]) -> Result<usize, Error>>(
		&mut self,
		mut buf: &[i16],
		write: F
	) -> Result<usize, Error> {
		let (rate, chan) = self.descriptor()?;

		let mut count = 0;
		while !buf.is_empty() {
			let mut frame = self.pop_frame(rate, chan);
			let n = write(&mut frame, buf)?;
			self.buf.push(frame);
			buf = &buf[n * chan..];
			count += n;
		}
		Ok(count)
	}
}

impl Default for PcmBuffer {
	fn default() -> Self { Self::new(FRAME_LEN) }
}

impl PcmStream for PcmBuffer {
	fn channel_count(&self) -> usize {
		self.descriptor()
			.map(|(_, c)| c)
			.unwrap_or_default()
	}

	fn sample_rate(&self) -> u32 {
		self.descriptor()
			.map(|(r, _)| r)
			.unwrap_or_default()
	}
}

impl PcmSource for PcmBuffer {
	fn read(&mut self, buf: &mut impl PcmSink, sample_count: usize) -> Result<usize, Error> {
		if sample_count == 0 { return Ok(0) }

		let mut count = 0;
		while let Some(mut frame) = (count < sample_count)
			.and_then(|| self.buf.pop()) {
			count += frame.read(buf, count)?;

			if !frame.is_empty() {
				self.buf.push(frame)
			}
		}
		Ok(count)
	}

	fn sample_count(&self) -> usize { self.len() }
}

impl PcmSink for PcmBuffer {
	fn write(&mut self, buf: &[i16], chn: usize) -> Result<usize, Error> {
		self.write_with(buf, |frame, buf| frame.write(buf, chn))
	}

	fn write_interleaved(&mut self, buf: &[i16]) -> Result<usize, Error> {
		self.write_with(buf, PcmFrame::write_interleaved)
	}

	fn write_frame(&mut self, frame: PcmFrame) -> Result<Option<PcmFrame>, Error> {
		if let Some(frame) = self.buf.last_mut() {
			frame.trim();
		}

		self.buf.push(frame);
		Ok(None)
	}

	fn sample_capacity(&self) -> usize { usize::MAX }

	fn set_descriptor(&mut self, sample_rate: u32, channel_count: usize) -> Result<(), Error> {
		if let Some(frame) = self.buf.last_mut() {
			if frame.is_full()             ||
			   sample_rate   != frame.rate ||
			   channel_count != frame.chan {
				self.new_frame(sample_rate, channel_count);
			}
		} else {
			self.new_frame(sample_rate, channel_count);
		}
		Ok(())
	}
}
