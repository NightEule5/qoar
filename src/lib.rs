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
#![feature(
	buf_read_has_data_left,
	generic_const_exprs,
	never_type,
)]

use std::{error, io, mem};
use std::cmp::min;
use std::fmt::Debug;
use amplify_derive::Display;

pub use encoder::*;
pub use decoder::*;
pub use pcm_io::*;

#[cfg(feature = "conv")]
pub mod conv;
mod pcm_io;
mod encoder;
mod decoder;

// Error

pub(crate) type Result<T = (), E = Error> = std::result::Result<T, E>;

#[derive(Debug, Display)]
pub enum Error {
	#[display("invalid stream descriptor ({0}); use streaming mode if unknown")]
	InvalidDescriptor(DescriptorError),
	#[display("stream descriptor cannot be set in a fixed encoder")]
	InvalidDescriptorChange,
	#[display("could not read samples")]
	SampleRead(Box<dyn error::Error>),
	#[display("could not {0}")]
	Write(WriteKind, io::Error),
	#[display("closed")]
	Closed,
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
			Self::SampleRead(err) => Some(err.as_ref()),
			Self::Write(_, err) => Some(err),
			_ => None
		}
	}
}

#[derive(Copy, Clone, Debug, Display)]
pub enum DescriptorError {
	#[display("sample rate {0} is outside the range the accepted range, [1,2^24)")]
	UnsupportedRate(u32),
	#[display("QOA streams must have at least one channel")]
	NoChannels,
	#[display("QOA streams must have at least one sample")]
	NoSamples,
	#[display("QOA streams are limited to 255 channels, but was {0}")]
	TooManyChannels(usize),
	#[display("QOA streams are limited to 2^32-1 samples, but was {0}")]
	TooManySamples(usize),
}

impl error::Error for DescriptorError { }

#[derive(Copy, Clone)]
pub struct StreamDescriptor {
	/// The number of samples per channel.
	sample_count: Option<u32>,
	/// The sample rate.
	sample_rate: Option<u32>,
	/// The number of channels.
	channel_count: Option<u8>,
	/// Whether the sample data is channel-interleaved.
	interleaved: bool,
}

impl StreamDescriptor {
	/// Creates a new stream descriptor. Fields can be omitted to infer with usage.
	///
	/// # Errors
	///
	/// [`DescriptorError::UnsupportedRate`]: `sample_rate` is outside the range
	/// `[1,2^24)`.
	///
	/// [`DescriptorError::NoChannels`]: `channel_count` is `0`.
	///
	/// [`DescriptorError::NoSamples`]: `sample_count` is `0`.
	fn new(
		sample_count: Option<u32>,
		sample_rate: Option<u32>,
		channel_count: Option<u8>,
		interleaved: bool,
	) -> Result<Self, DescriptorError> {
		if let Some(rate) = sample_rate {
			if !(1..16777216).contains(&rate) {
				return Err(DescriptorError::UnsupportedRate(rate))
			}
		}

		if let Some(_s @ 0) = sample_count {
			return Err(DescriptorError::NoSamples)
		}

		if let Some(_c @ 0) = channel_count {
			return Err(DescriptorError::NoChannels)
		}

		Ok(Self {
			sample_count,
			sample_rate,
			channel_count,
			interleaved,
		})
	}

	pub fn samples(&self) -> Option<u32> { self.sample_count }
	pub fn rate(&self) -> Option<u32> { self.sample_rate }
	pub fn channels(&self) -> Option<u8> { self.channel_count }

	pub fn suggest_sample_count(&mut self, sample_count: u32) {
		let samples = self.sample_count.get_or_insert(sample_count);
		*samples = min(*samples, sample_count);
	}

	pub fn suggest_sample_rate(&mut self, sample_rate: u32) {
		let _ = self.sample_rate.get_or_insert(sample_rate);
	}

	pub fn suggest_channel_count(&mut self, channel_count: u8) {
		let _ = self.channel_count.get_or_insert(channel_count);
	}
	
	fn is_streaming(&self) -> bool {
		self.sample_count.is_none()
	}

	fn unwrap_all(self) -> (u32, u32, u8, bool) {
		let Self { sample_count, sample_rate, channel_count, interleaved } = self;

		(
			sample_count .unwrap_or_default(),
			sample_rate  .unwrap_or_default(),
			channel_count.unwrap_or_default(),
			interleaved
		)
	}

	fn set<T: Copy>(option: &mut Option<T>, fallback: &Option<T>)  {
		if option.is_none() {
			if let Some(value) = fallback {
				let _ = option.insert(*value);
			}
		}
	}

	pub(crate) fn infer_from_vec(&mut self, vec: &Vec<i16>, fallback: &Self) {
		self.infer(fallback);

		if let Some(samples) = self.sample_count.as_mut() {
			// Infer channel count from sample count.
			let _ = self.channel_count.get_or_insert_with(|| {
				*samples = min(*samples, vec.len() as u32);
				(vec.len() as u32 / *samples) as u8
			});
		} else if let Some(chn) = self.channel_count {
			// Infer sample count from channel count.
			let _ = self.sample_count.get_or_insert_with(||
				vec.len() as u32 / chn as u32
			);
		}
	}

	pub(crate) fn infer(&mut self, fallback: &Self) {
		Self::set(&mut self.sample_count,  &fallback.sample_count );
		Self::set(&mut self.sample_rate,   &fallback.sample_rate  );
		Self::set(&mut self.channel_count, &fallback.channel_count);

		if let Some(_s @ 0) = self.sample_rate {
			self.sample_rate = None;
		}

		if let Some(_c @ 0) = self.sample_rate {
			self.channel_count = None;
		}
	}
}

impl Default for StreamDescriptor {
	fn default() -> Self {
		Self {
			sample_count: None,
			sample_rate: None,
			channel_count: None,
			interleaved: true,
		}
	}
}

#[derive(Copy, Clone)]
struct QoaLmsState {
	history: [i32; 4],
	weights: [i32; 4],
}

#[derive(Copy, Clone, Default)]
struct QoaSlice {
	quant: u8,
	resid: [u8; 20],
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
			*weight += if history < 0 { -delta } else { delta } as i32;
		}

		self.history.rotate_left(1);
		self.history[3] = sample as i32;
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
