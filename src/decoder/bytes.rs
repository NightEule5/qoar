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

use std::assert_matches::debug_assert_matches;
use std::cmp::min;
use std::result;
use std::io::Read;
use amplify_derive::{Display, Error};
use Error::{Eos, UnknownMagic};
use crate::{DEQUANT_TABLE, MAGIC, SLICE_LEN};
use crate::byte_decoder::Error::DescriptorChange;
use crate::util::Zip;

type Result<T = ()> = result::Result<T, Error>;

#[derive(Debug, Display, Error)]
pub enum Error {
	#[display("unknown magic bytes {0:?}")]
	UnknownMagic([u8; 4]),
	#[display(
		"variable sample rate or channel count is unsupported, attempted to change \
		sample rate to {0} and channel to {1}"
	)]
	DescriptorChange(u32, usize),
	#[display("{0}")]
	IO(crate::Error),
	#[display("unexpected end-of-stream")]
	Eos,
}

impl From<crate::Error> for Error {
	fn from(value: crate::Error) -> Self { Self::IO(value) }
}

#[derive(Clone, Debug, Default)]
pub struct Decoder {
	lms: Vec<LmsState>,
	buf: [i16; SLICE_LEN],
}

impl Decoder {
	pub fn decode(&mut self, mut source: &[u8], sink: &mut Vec<i16>) -> Result<usize> {
		let mut sample_count = source.decode_file_header()? as usize;
		let streaming_mode = sample_count == 0;

		let mut sample_rate = 0;
		let mut channels    = 0;

		let mut bytes = 8;
		while sample_count > 0 || (streaming_mode && !source.is_empty()) {
			let (chan, rate, samples, _) = source.decode_frame_header()?;

			if sample_rate == 0  {
				sample_rate = rate;
			} else if sample_rate != rate {
				return Err(DescriptorChange(rate, chan))
			}

			if channels == 0 {
				channels = chan;
			} else if channels != chan {
				return Err(DescriptorChange(rate, chan))
			}

			let size = self.decode_frame(source, sink, samples, chan)?;
			source = &source[size..];
			bytes += size + 8;
			sample_count -= samples;
		}

		Ok(bytes)
	}

	fn decode_frame(
		&mut self,
		mut source: &[u8],
		sink: &mut Vec<i16>,
		samples: usize,
		channels: usize,
	) -> Result<usize> {
		let Self { ref mut lms, ref mut buf } = self;
		lms.resize_with(channels, Default::default);
		source.decode_lms(lms)?;

		let slices = min(samples / SLICE_LEN, 256);

		for _ in 0..slices {
			for chn in 0..channels {
				let ref mut lms = lms[chn];
				let mut slice = source.read_long()?;
				let len = min(SLICE_LEN, samples);
				let sf = ((slice >> 60) & 0xF) as usize;

				for si in 0..len {
					let qr = ((slice >> 57) & 0x7) as usize;
					slice <<= 3;
					let dq = DEQUANT_TABLE[sf][qr];
					let pr = lms.predict();
					let re = (pr + dq).clamp(-32768, 32767) as i16;

					buf[si] = re;

					lms.update(re, dq);
				}

				sink.extend_from_slice(&buf[..len]);
			}
		}

		Ok(8 * (channels * 2 + slices))
	}
}

trait Source: Read {
	fn read_long(&mut self) -> Result<u64> {
		let mut bytes = [0; 8];

		// The only error returned from the read_exact implementation on slices is
		// Eof, so this is fine.
		self.read_exact(&mut bytes).map_err(|_| Eos)?;

		Ok(u64::from_be_bytes(bytes))
	}

	fn read_longs<const N: usize>(&mut self) -> Result<[u64; N]> {
		let mut longs = [0; N];
		for long in &mut longs {
			*long = self.read_long()?
		}

		Ok(longs)
	}

	fn decode_file_header(&mut self) -> Result<u32> {
		let value = self.read_long()?;
		let magic = (value >> 32) as u32;

		if magic != MAGIC {
			return Err(UnknownMagic(magic.to_be_bytes()))
		}

		Ok(value as u32)
	}

	fn decode_frame_header(&mut self) -> Result<(usize, u32, usize, usize)> {
		let value    = self.read_long()?;
		let channels = (value >> 56) as u8  as usize;
		let rate     = (value >> 32) as u32 & 0xFFFFFF;
		let samples  = (value >> 16) as u16 as usize;
		let size     = (value >>  0) as u16 as usize;
		Ok((channels, rate, samples, size))
	}

	fn decode_lms(&mut self, lms: &mut [LmsState]) -> Result {
		for lms in lms { lms.unpack(self.read_longs()?) }
		Ok(())
	}
}

impl Source for &[u8] { }

#[derive(Copy, Clone, Debug, Default)]
struct LmsState {
	history: [i16; 4],
	weights: [i16; 4]
}

impl LmsState {
	fn unpack(&mut self, [mut history, mut weights]: [u64; 2]) {
		self.history.fill_with(|| {
			let val = (history >> 48) as i16;
			history <<= 16;
			val
		});
		self.weights.fill_with(|| {
			let val = (weights >> 48) as i16;
			weights <<= 16;
			val
		})
	}

	fn predict(&self) -> i32 {
		let history = self.history.into_iter().map(|h| h as i32);
		let weights = self.weights.into_iter().map(|w| w as i32);
		history.zip(weights).mul().sum::<i32>() >> 13
	}

	fn update(&mut self, sample: i16, residual: i32) {
		debug_assert_matches!(
			residual >> 4,
			r @ -32768..=32767,
			"residual larger than expected"
		);
		let delta = (residual >> 4) as i16;

		for i in 0..4 {
			self.weights[i] = if self.history[i] < 0 { -delta } else { delta };
		}

		self.history.copy_within(1..4, 0);
		self.history[3] = sample;
	}
}
