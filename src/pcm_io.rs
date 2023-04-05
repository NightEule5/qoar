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

use std::{error, io};
use std::cmp::min;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use crate::{DescriptorError, StreamDescriptor};

// Source

/// A raw PCM-S16LE audio source.
pub trait Pcm16Source {
	type Error: error::Error + Into<Box<dyn error::Error>>;

	/// Reads channel-interleaved data into the buffer slice, returning the number
	/// of samples read. This buffer's length should be a multiple of the channels.
	fn read_interleaved(&mut self, buf: &mut [i16]) -> Result<usize, Self::Error>;

	/// Returns true if the source has no more samples.
	fn exhausted(&mut self) -> Result<bool, Self::Error>;

	/// Returns a [`StreamDescriptor`].
	fn descriptor(&self) -> Result<StreamDescriptor, DescriptorError>;
}

pub trait IntoSource {
	type Source: Pcm16Source;

	/// Converts into a [`Pcm16Source`] with the suggested stream descriptor. There
	/// is no guarantee that this will be used.
	fn into_source(self, descriptor: StreamDescriptor) -> Self::Source;
}

impl<R: Read> IntoSource for BufReader<R> {
	type Source = ReadSource<R>;

	fn into_source(self, desc: StreamDescriptor) -> Self::Source {
		ReadSource { source: self, desc }
	}
}

impl IntoSource for Vec<i16> {
	type Source = VecSource;

	fn into_source(self, mut descriptor: StreamDescriptor) -> Self::Source {
		descriptor.suggest_sample_count(self.len() as u32);
		VecSource { source: self, desc: descriptor }
	}
}

impl IntoSource for File {
	type Source = ReadSource<File>;

	fn into_source(self, mut desc: StreamDescriptor) -> Self::Source {
		if let Some(channels) = desc.channel_count {
			if let Ok(metadata) = self.metadata() {
				desc.suggest_sample_count(
					(metadata.len() / channels as u64) as u32
				);
			}
		}

		BufReader::new(self).into_source(desc)
	}
}

/// Reads from a [`Vec`] of channel-interleaved samples.
pub struct VecSource {
	source: Vec<i16>,
	desc: StreamDescriptor
}

impl VecSource {
	fn len(&self) -> usize {
		let len = self.source.len();
		len - len % self.desc.channel_count.unwrap() as usize
	}
}

impl Pcm16Source for VecSource {
	type Error = !;

	fn read_interleaved(&mut self, buf: &mut [i16]) -> Result<usize, !> {
		let n = min(self.len(), buf.len());
		buf.copy_from_slice(&self.source[..n]);
		Ok(n)
	}

	fn exhausted(&mut self) -> Result<bool, !> { Ok(self.len() == 0) }

	fn descriptor(&self) -> Result<StreamDescriptor, DescriptorError> { Ok(self.desc) }
}

/// Reads from a raw [`Read`] stream of channel-interleaved samples.
pub struct ReadSource<R: Read> {
	source: BufReader<R>,
	desc: StreamDescriptor
}

impl<R: Read> Pcm16Source for ReadSource<R> {
	type Error = io::Error;

	fn read_interleaved(&mut self, buf: &mut [i16]) -> Result<usize, Self::Error> {
		let len = buf.len() - buf.len() % self.desc.channel_count.unwrap() as usize;
		let mut n = 0;
		while n < len && !self.exhausted()? {
			let len = len - n;
			let samples = self.source
							  .buffer()
							  .chunks(2)
							  .take(len)
							  .map(|bytes| {
								  let bytes = [bytes[0], bytes[1]];
								  i16::from_le_bytes(bytes)
							  })
							  .enumerate();
			for (i, sample) in samples {
				buf[i] = sample;
				n += 1;
			}
		}
		Ok(n)
	}

	fn exhausted(&mut self) -> Result<bool, Self::Error> {
		Ok(!self.source.has_data_left()?)
	}

	fn descriptor(&self) -> Result<StreamDescriptor, DescriptorError> { Ok(self.desc) }
}

// Sink

pub trait Pcm16Sink {
	type Error: error::Error + Into<Box<dyn error::Error>>;

	/// Writes samples from `buf` into `chn`.
	fn write(&mut self, buf: &[i16], chn: u8) -> Result<(), Self::Error>;

	/// Sets the `sample_rate` and `channel_count`.
	fn set_descriptor(
		&mut self,
		sample_rate: u32,
		channel_count: u8
	) -> Result<(), Self::Error>;

	/// Flushes buffered samples.
	fn flush(&mut self) -> Result<(), Self::Error> { Ok(()) }
	/// Closes the sink.
	fn close(&mut self) -> Result<(), Self::Error> { self.flush() }
}
