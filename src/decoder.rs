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

use std::error::Error;
use std::result;
use std::cmp::min;
use amplify_derive::Display;
use crate::{DEQUANT_TABLE, MAGIC, PcmSink, QoaLmsState, QoaSlice, SLICE_LEN};

use DecodeError::*;
use DecodeWriteKind::*;
use crate::io::{ReadError, SourceStream};

pub mod bytes;

type Result<T = ()> = result::Result<T, DecodeError>;

#[derive(Debug, Display)]
pub enum DecodeError {
	#[display("unknown magic byte sequence {0:?}")]
	UnknownMagic([u8; 4]),
	#[display("end of stream reached prematurely")]
	Eof,
	#[display("could not read data ({0})")]
	Read(ReadError),
	#[display("could not {0} sink")]
	Write(DecodeWriteKind, Box<dyn Error>),
	#[display("could not close sink")]
	SinkClose(Box<dyn Error>),
}

#[derive(Copy, Clone, Debug, Display)]
pub enum DecodeWriteKind {
	#[display("set sample rate and channel count in")]
	SetDescriptor,
	#[display("write sample to")]
	Sample,
}

impl Error for DecodeError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			Read(ref inner) => Some(inner),
			Write(_, inner) |
			SinkClose(inner) => Some(inner.as_ref()),
			_ => None
		}
	}
}

impl From<ReadError> for DecodeError {
	fn from(value: ReadError) -> Self {
		if let ReadError::Eof = value {
			Eof
		} else {
			Read(value)
		}
	}
}

pub struct Decoder<S: PcmSink> {
	samples: Option<u32>,
	sink: S,
	header: bool,
	lms: Vec<QoaLmsState>,
	slice: QoaSlice,
	slice_buf: [i16; SLICE_LEN],
}

impl<Sn: PcmSink> Decoder<Sn> {
	pub fn new(sink: Sn) -> Self {
		Self {
			samples: None,
			sink,
			header: true,
			lms: Vec::new(),
			slice: QoaSlice::default(),
			slice_buf: [0; SLICE_LEN],
		}
	}
	
	/// Decodes all samples from a QOA `source`, returning the underlying sink.
	pub fn decode<S: SourceStream>(mut self, source: &mut S) -> Result<Sn> {
		while self.decode_frame(source)? { }
		self.close()
	}

	/// Decodes a QOA frame from `source`, returning `true` if a frame was decoded.
	pub fn decode_frame<S: SourceStream>(&mut self, source: &mut S) -> Result<bool> {
		let Self { samples, sink, header, lms, slice, slice_buf } = self;
		let streaming_mode;
		let samples = {
			if *header {
				*header = false;
				let header_samples = source.dec_file_header()?;
				streaming_mode = header_samples == 0;

				if !streaming_mode {
					let _ = samples.insert(header_samples);
				}

				header_samples
			} else {
				streaming_mode = samples.is_none();
				samples.unwrap_or_default()
			}
		};

		if samples == 0 && !streaming_mode {
			return Ok(false)
		}

		let (channels, rate, f_samples, _) = {
			let header = source.dec_frame_header();

			// The EOF isn't an error in streaming mode, it's the break signal for
			// frame decoding. In contrast with fixed mode, where ending before we
			// read the number of samples given in the header is an error.
			if streaming_mode {
				if let Err(Eof) = header {
					return Ok(false)
				}
			}

			header?
		};

		lms.resize_with(channels as usize, Default::default);
		source.dec_lms(lms)?;

		sink.set_descriptor(rate, channels as usize)
			.map_err(|err| Write(SetDescriptor, err.into()))?;

		for sample in (0..f_samples).step_by(SLICE_LEN) {
			let slice_width = min(SLICE_LEN, (f_samples - sample) as usize);
			for chn in 0..channels {
				source.dec_slice(slice)?;

				let QoaSlice { quant, resid } = slice;

				for si in 0..slice_width {
					let qr = resid[si];
					let predicted = lms[chn as usize].predict();
					let dequantized = DEQUANT_TABLE[*quant as usize][qr as usize];
					let reconst = (predicted + dequantized) as i16;

					slice_buf[si] = reconst;

					lms[chn as usize].update(reconst, dequantized);
				}

				sink.write(&slice_buf[..slice_width], chn as usize)
					.map_err(|err| Write(Sample, err.into()))?;
			}
		}

		self.sub_samples(f_samples as u32);
		Ok(true)
	}
	
	/// Flushes and closes the underlying sink, then returns it.
	pub fn close(mut self) -> Result<Sn> {
		self.sink
			.close()
			.map_err(|err| SinkClose(err.into()))?;
		Ok(self.sink)
	}

	fn sub_samples(&mut self, n: u32) {
		if let Some(ref mut samples) = self.samples {
			*samples = (*samples).saturating_sub(n);
		}
	}
}

impl<S: PcmSink> From<S> for Decoder<S> {
	fn from(value: S) -> Self { Self::new(value) }
}

pub(crate) trait QoaSource: SourceStream {
	fn dec_file_header(&mut self) -> Result<u32> {
		let v = self.read_long()?;

		let magic = (v >> 32) as u32;
		if magic != MAGIC {
			return Err(UnknownMagic(magic.to_be_bytes()))
		}

		Ok(v as u32)
	}

	fn dec_frame_header(&mut self) -> Result<(u8, u32, u16, u16)> {
		let v = self.read_long()?;
		let channels = (v >> 56) as u8;
		let rate = ((v >> 32) & 0xFFFFFF) as u32;
		let samples = (v >> 16) as u16;
		let size = v as u16;
		Ok((channels, rate, samples, size))
	}

	fn dec_lms(&mut self, lms: &mut [QoaLmsState]) -> Result {
		for lms in lms {
			let mut history = self.read_long()?;
			let mut weights = self.read_long()?;
			for i in 0..4 {
				lms.history[i] = (history >> 48) as i16 as i32;
				history <<= 16;
				lms.weights[i] = (weights >> 48) as i16 as i32;
				weights <<= 16;
			}
		}
		Ok(())
	}

	fn dec_slice(&mut self, slice: &mut QoaSlice) -> Result {
		slice.unpack(self.read_long()?);
		Ok(())
	}
}

impl<S: SourceStream> QoaSource for S { }

impl QoaSlice {
	fn unpack(&mut self, mut v: u64) {
		let ref mut resid = self.resid;
		for i in (0..20).rev() {
			resid[i] = (v & 0b111) as u8;
			v >>= 3;
		}
		self.quant = v as u8;
	}
}

#[cfg(test)]
mod test {
	use crate::QoaSlice;

	#[test]
	fn unpack_slice() {
		// q=9, r00=1, r01=2, ..., r19=6
		const PACKED: u64 = 0b1001_001_010_011_100_101_110_111_001_010_011_100_101_110_111_001_010_011_100_101_110;
		const UNPACKED: [u8; 20] = {
			let mut arr = [0; 20];
			let mut i = 0;
			let mut v = 1;

			while i < 20 {
				arr[i] = v;

				i += 1;
				v += 1;
				if v == 8 {
					v = 1;
				}
			}

			arr
		};

		let mut slice = QoaSlice::default();
		slice.unpack(PACKED);

		assert_eq!(slice, QoaSlice { quant: 9, resid: UNPACKED });
	}
}
