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

use std::{error, io, mem};
use std::cmp::min;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::ops::{Deref, DerefMut};
use amplify_derive::Display;
use crate::{SLICE_LEN, StreamDescriptor};

// Stream traits

/// A stream of PCM samples.
pub trait PcmStream {
	/// Returns the current channel count of the stream, or `0` if not known.
	fn channel_count(&mut self) -> u8;
	/// Returns the current sample rate of the stream, or `0` if not known.
	fn sample_rate(&mut self) -> u32;
}

#[derive(Debug, Display)]
pub enum Error {
	#[display("cannot set descriptor of fixed sink")]
	DescriptorSet,
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

		let mut cnt = self.sample_count(chan);
		while self.read(&mut buf, cnt)? > 0 {
			cnt = self.sample_count(chan);
		}

		Ok(buf)
	}

	/// Returns the number of samples per channel available, or `0` if not known.
	/// A `channel_count` is suggested, but not necessarily used, to calculate the
	/// count. If the channel count is not known and the suggested count is `0`,
	/// the returned sample count will be the total count.
	fn sample_count(&mut self, channel_count: u8) -> usize;

	fn descriptor(&mut self) -> StreamDescriptor {
		let samples  = self.sample_count(0) as u32;
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
	fn write(&mut self, buf: &[i16], chn: u8) -> Result<usize, Error>;

	/// Writes interleaved samples from `buf`, then returns the number of slices
	/// written, or the total number of samples written if the channel count isn't
	/// known.
	fn write_interleaved(&mut self, buf: &[i16]) -> Result<usize, Error>;

	/// Sets the `sample_rate` and `channel_count`.
	fn set_descriptor(
		&mut self,
		sample_rate: u32,
		channel_count: u8
	) -> Result<(), Error>;

	/// Flushes buffered samples.
	fn flush(&mut self) -> Result<(), Error> { Ok(()) }
	/// Closes the sink.
	fn close(&mut self) -> Result<(), Error> { self.flush() }
}

// Buffer

/// A buffer of interleaved PCM samples.
#[derive(Clone, Debug, Default)]
pub struct PcmBuffer {
	buf: Vec<i16>,
	len: Vec<usize>,
	rate: u32,
	chan: usize,
}

impl PcmBuffer {
	pub fn new(sample_count: usize, sample_rate: u32, channel_count: u8) -> Self {
		let chan = channel_count as usize;
		Self {
			buf: vec![0; sample_count * chan],
			len: vec![0; chan],
			rate: sample_rate,
			chan,
		}
	}

	pub fn len(&self) -> usize { self.len.iter().sum() }

	pub fn is_empty(&self) -> bool { self.len() == 0 }

	pub fn clear(&mut self) {
		self.buf.clear();

		for len in self.len.iter_mut() {
			*len = 0;
		}
	}

	pub fn unwrap(mut self) -> Vec<i16> {
		let len = self.len();
		// Pad with silence if the channels aren't the same length.
		self.buf.resize(len + ((len % self.chan != 0) as usize * self.chan), 0);
		self.buf
	}

	pub fn encode(self) -> Vec<u8> {
		let samples = self.unwrap();
		let mut buf = Vec::with_capacity(samples.len() * 2);

		for sample in samples {
			buf.extend_from_slice(&sample.to_le_bytes());
		}

		buf
	}

	fn reserve(&mut self, samples: usize) {
		self.reserve_exact(samples * self.chan);
	}

	fn reserve_exact(&mut self, total_samples: usize) {
		self.buf.resize(self.buf.len() + total_samples - total_samples % self.chan, 0);
	}

	fn truncate(&mut self, samples: usize) {
		let Self { buf, len, chan, .. } = self;

		buf.truncate(buf.len().saturating_sub(samples * *chan));

		for len in len.iter_mut() {
			*len = len.saturating_sub(samples);
		}
	}
}

impl Deref for PcmBuffer {
	type Target = Vec<i16>;

	fn deref(&self) -> &Self::Target { &self.buf }
}

impl DerefMut for PcmBuffer {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.buf
	}
}

impl PcmStream for PcmBuffer {
	fn channel_count(&mut self) -> u8 { self.chan as u8 }

	fn sample_rate(&mut self) -> u32 { self.rate }
}

impl PcmSource for PcmBuffer {
	fn read(&mut self, buf: &mut impl PcmSink, sample_count: usize) -> Result<usize, Error> {
		buf.set_descriptor(self.rate, self.chan as u8)?;
		let len = min(self.len(), sample_count * self.chan);
		let read = buf.write_interleaved(&self[..len])?;
		self.truncate(read);
		Ok(read)
	}

	fn sample_count(&mut self, _: u8) -> usize { self.len() }
}

impl PcmSink for PcmBuffer {
	fn write(&mut self, buf: &[i16], chn: u8) -> Result<usize, Error> {
		let chn = chn as usize;
		if chn >= self.chan { return Ok(0) }

		self.reserve(buf.len());

		let off = self.len[chn] * self.chan + chn;
		self.len[chn] += buf.len();
		for i in 0..buf.len() {
			self.buf[off + i] = buf[i];
		}

		Ok(buf.len())
	}

	fn write_interleaved(&mut self, buf: &[i16]) -> Result<usize, Error> {
		self.reserve_exact(buf.len());
		let ref mut lengths = self.len;
		for i in (0..buf.len()).step_by(self.chan) {
			for chn in 0..self.chan {
				let off = lengths[chn] + chn;
				self.buf[off + i] = buf[i];
			}
		}
		self.buf.extend_from_slice(buf);

		let chn_len = buf.len() / self.chan;
		for len in self.len.iter_mut() {
			*len += chn_len;
		}
		Ok(chn_len)
	}

	fn set_descriptor(&mut self, sample_rate: u32, channel_count: u8) -> Result<(), Error> {
		if sample_rate > 0 && self.rate == 0 && self.is_empty() {
			self.rate = sample_rate;
		}

		if channel_count > 0 && self.chan == 0 && self.is_empty() {
			self.chan = channel_count as usize;
			self.len.resize(self.chan, 0);
		}

		if (sample_rate   == 0 || self.rate == sample_rate) &&
		   (channel_count == 0 || self.chan == channel_count as usize) {
			Ok(())
		} else {
			Err(Error::DescriptorSet)
		}
	}
}

// IO

trait Source: Read {
	fn read_i16(&mut self) -> io::Result<Option<i16>> {
		let mut bytes = [0; 2];
		if self.read(&mut bytes)? == 2 {
			Ok(Some(i16::from_le_bytes(bytes)))
		} else {
			Ok(None)
		}
	}
}

impl<R: Read> Source for R { }

const SAMPLE_LEN: usize = mem::size_of::<i16>();

default impl<R: Read> PcmStream for BufReader<R> {
	fn channel_count(&mut self) -> u8 { 0 }

	fn sample_rate(&mut self) -> u32 { 0 }
}

default impl<R: Read + Seek> PcmSource for BufReader<R> {
	default fn read(
		&mut self,
		buf: &mut impl PcmSink,
		sample_count: usize
	) -> Result<usize, Error> {
		impl From<io::Error> for Error {
			fn from(value: io::Error) -> Self { Self::Read(value.into()) }
		}

		if sample_count == 0 { return Ok(0) }

		let mut chan = buf.channel_count() as usize;
		if chan == 0 { chan = 1; }
		let len = sample_count * chan;

		let mut samples = Vec::with_capacity(len);
		while let Some(sample) = self.read_i16()? {
			samples.push(sample);

			if samples.len() >= len {
				break
			}
		}
		buf.write_interleaved(&samples)
	}

	default fn sample_count(&mut self, mut channel_count: u8) -> usize {
		if channel_count < 1 {
			channel_count = 1;
		}

		let byte_count = self.stream_len()
							 .unwrap_or_default() as usize;
		byte_count / SAMPLE_LEN / channel_count as usize
	}
}

impl PcmStream for BufReader<File> { }

impl PcmSource for BufReader<File> {
	fn sample_count(&mut self, mut channel_count: u8) -> usize {
		if channel_count < 1 {
			channel_count = 1;
		}

		let byte_count = self.get_ref()
							 .metadata()
							 .map(|meta| meta.len())
							 .unwrap_or_default() as usize;
		byte_count / SAMPLE_LEN / channel_count as usize
	}
}

impl<W: Write> PcmStream for BufWriter<W> {
	fn channel_count(&mut self) -> u8 { 0 }

	fn sample_rate(&mut self) -> u32 { 0 }
}

impl<W: Write> PcmSink for BufWriter<W> {
	fn write(&mut self, _buf: &[i16], _chn: u8) -> Result<usize, Error> {
		todo!("Planar buffered write operation not yet implemented")
	}

	fn write_interleaved(&mut self, buf: &[i16]) -> Result<usize, Error> {
		for sample in buf {
			self.write_all(&sample.to_le_bytes())
				.map_err(|err| Error::Write(err.into()))?
		}

		Ok(buf.len())
	}

	fn set_descriptor(&mut self, _sample_rate: u32, _channel_count: u8) -> Result<(), Error> {
		Ok(())
	}
}
