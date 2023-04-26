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
	assert_matches,
	associated_type_defaults,
	buf_read_has_data_left,
	generic_const_exprs,
	iter_array_chunks,
	never_type,
	return_position_impl_trait_in_trait,
	seek_stream_len,
	slice_flatten,
	specialization,
)]
#![cfg(feature = "simd")]
#![feature(portable_simd)]

use std::cmp::min;
use amplify_derive::{Display, Error};

pub use encoder::*;
pub use decoder::bytes as byte_decoder;
pub use pcm_io::*;

#[cfg(feature = "conv")]
pub mod conv;
mod pcm_io;
mod encoder;
mod decoder;
pub mod io;
mod util;
mod simd;

#[derive(Copy, Clone, Debug, Display, Error)]
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

#[derive(Copy, Clone)]
pub struct StreamDescriptor {
	/// The number of samples per channel.
	sample_count: Option<usize>,
	/// The sample rate.
	sample_rate: Option<u32>,
	/// The number of channels.
	channel_count: Option<usize>,
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
		sample_count: Option<usize>,
		sample_rate: Option<u32>,
		channel_count: Option<usize>,
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
		})
	}

	pub fn samples(&self) -> Option<usize> { self.sample_count }
	pub fn rate(&self) -> Option<u32> { self.sample_rate }
	pub fn channels(&self) -> Option<usize> { self.channel_count }

	pub fn suggest_sample_count(&mut self, sample_count: usize) {
		let samples = self.sample_count.get_or_insert(sample_count);
		*samples = min(*samples, sample_count);
	}

	pub fn suggest_sample_rate(&mut self, sample_rate: u32) {
		let _ = self.sample_rate.get_or_insert(sample_rate);
	}

	pub fn suggest_channel_count(&mut self, channel_count: usize) {
		let _ = self.channel_count.get_or_insert(channel_count);
	}
	
	fn is_streaming(&self) -> bool {
		self.sample_count.is_none()
	}

	fn unwrap_all(self) -> (usize, u32, usize) {
		let Self { sample_count, sample_rate, channel_count } = self;

		(
			sample_count .unwrap_or_default(),
			sample_rate  .unwrap_or_default(),
			channel_count.unwrap_or_default(),
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
				*samples = min(*samples, vec.len());
				vec.len() / *samples
			});
		} else if let Some(chn) = self.channel_count {
			// Infer sample count from channel count.
			let _ = self.sample_count.get_or_insert_with(|| vec.len() / chn);
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
		}
	}
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct QoaLmsState {
	history: [i32; 4],
	weights: [i32; 4],
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
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

const RECIP_TABLE: [i64; 16] = [
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
		let history = self.history.iter().cloned();
		let weights = self.weights.iter().cloned();
		let p: i32 = history.zip(weights).map(|(h, w)| h * w).sum();
		p >> 13
	}

	fn update(&mut self, sample: i16, residual: i32) {
		let delta = residual >> 4;
		for (history, weight) in self.history
									 .into_iter()
									 .zip(self.weights.iter_mut()) {
			*weight += if history < 0 { -delta } else { delta };
		}

		self.history.copy_within(1..4, 0);
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
	let mut n = ((v as i64 * recip + (1 << 15)) >> 16) as i32;
	n += ((v > 0) as i32 - (v < 0) as i32) -
		 ((n > 0) as i32 - (n < 0) as i32);
	n
}

#[cfg(test)]
mod test {
	use std::fmt::{Arguments, Debug};
	use quickcheck_macros::quickcheck;
	use quickcheck::{Arbitrary, Gen, TestResult};
	use qoa_ref_sys::qoa::qoa_lms_t;
	use crate::decoder::QoaSource;
	use crate::encoder::QoaSink;
	use crate::io::Buffer;
	use crate::{DEQUANT_TABLE, QoaLmsState};

	#[macro_export]
	macro_rules! qc_assert_eq {
		($left:expr,$right:expr) => {{
			let left = $left;
			let right = $right;
			if left == right {
				TestResult::passed()
			} else {
				TestResult::error(
					crate::test::format_qc_assert_error(&left, &right, None)
				)
			}
		}};
		($left:expr,$right:expr,$($arg:tt)+) => {{
			let left = $left;
			let right = $right;
			if left == right {
				TestResult::passed()
			} else {
				TestResult::error(
					crate::test::format_qc_assert_error(&left, &right, Some(format_args!($($arg)+)))
				)
			}
		}};
	}

	pub fn format_qc_assert_error<L: Debug, R: Debug>(left: &L, right: &R, msg: Option<Arguments>) -> String {
		let msg = msg.map(|msg| {
			let mut msg = msg.to_string();
			msg.insert(0, ' ');
			msg
		}).unwrap_or_default();

		format!(
			"assertion failed `(left == right)`:{msg}\n \
			left: `{left:?}`,\nright: `{right:?}`",
		)
	}

	impl Arbitrary for QoaLmsState {
		fn arbitrary(g: &mut Gen) -> Self {
			let mut history = [0; 4];
			let mut weights = [0; 4];

			history.fill_with(|| i16::arbitrary(g) as i32);
			weights.fill_with(|| i16::arbitrary(g) as i32);

			Self { history, weights }
		}
	}

	impl From<qoa_lms_t> for QoaLmsState {
		fn from(qoa_lms_t { history, weights }: qoa_lms_t) -> Self {
			QoaLmsState { history, weights }
		}
	}

	impl Into<qoa_lms_t> for QoaLmsState {
		fn into(self) -> qoa_lms_t {
			let Self { history, weights } = self;
			qoa_lms_t { history, weights }
		}
	}

	#[quickcheck]
	fn lms_predict(lms: QoaLmsState) -> TestResult {
		let exp = {
			let mut lms: qoa_lms_t = lms.into();
			lms.predict()
		};

		let act = lms.predict();
		qc_assert_eq!(act, exp)
	}

	#[quickcheck]
	fn lms_update(mut lms: QoaLmsState, sample: i16, residual: i32) -> TestResult {
		if !DEQUANT_TABLE.flatten().contains(&residual) {
			return TestResult::discard()
		}

		let mut other: qoa_lms_t = lms.into();
		other.update(sample, residual);
		let other = lms.into();
		lms.update(sample, residual);
		qc_assert_eq!(lms, other)
	}

	#[quickcheck]
	fn codec_file_header(sample_count: u32) -> TestResult {
		let mut buf = Buffer::default();
		if let Err(error) = buf.enc_file_header(sample_count as usize) {
			return TestResult::error(format!("{error}"))
		}

		match buf.dec_file_header() {
			Ok(samples) => qc_assert_eq!(samples, sample_count),
			Err(error) => TestResult::error(format!("{error}"))
		}
	}

	#[quickcheck]
	fn decode_frame_header(channels: u8, rate: u32, samples: u16, size: u16) -> TestResult {
		if channels == 0 || !(1..16777216).contains(&rate) || samples == 0 || size == 0 {
			return TestResult::discard()
		}

		let mut buf = Buffer::default();
		if let Err(error) = buf.enc_frame_header(channels as usize, rate, samples, size) {
			return TestResult::error(format!("{error}"))
		}

		match buf.dec_frame_header() {
			Ok(header) => qc_assert_eq!(header, (channels, rate, samples, size)),
			Err(error) => TestResult::error(format!("{error}"))
		}
	}

	#[quickcheck]
	fn codec_lms(lms: QoaLmsState) -> TestResult {
		let mut buf = Buffer::default();
		if let Err(error) = buf.enc_lms_state(&lms) {
			return TestResult::error(format!("{error}"))
		}

		let mut decoded = [QoaLmsState::default(); 1];
		if let Err(error) = buf.dec_lms(&mut decoded) {
			return TestResult::error(format!("{error}"))
		}

		qc_assert_eq!(lms, decoded[0])
	}
}