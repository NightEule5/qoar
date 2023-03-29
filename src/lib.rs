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
#![feature(iter_array_chunks)]
#![feature(buf_read_has_data_left)]
#![feature(array_windows)]

use std::{error, fmt, io, mem};
use std::fmt::{Debug, Formatter, write};
use std::io::{BufRead, BufReader, Read, Write};

#[cfg(test)]
mod test;

// Error

type Result<T = (), E = Error> = std::result::Result<T, E>;

#[derive(Clone, Debug)]
pub enum ErrorKind {
	InvalidRate(usize),
	Exhausted,
	UnknownMagic([u8; 4]),
	IO,
	SampleRead,
	ChannelCountRead,
}

#[derive(Debug)]
pub struct Error {
	message: Option<String>,
	kind: ErrorKind,
	source: Option<Box<dyn error::Error>>,
}

impl fmt::Display for ErrorKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidRate(rate) => write!(f, "invalid rate ({rate}), must be [1,16777215]"),
			Self::Exhausted => write!(f, "buffer exhausted"),
			Self::UnknownMagic(magic) => write!(f, "unknown magic ({magic:?})"),
			Self::IO => write!(f, "IO error"),
			Self::SampleRead => write!(f, "could not read sample"),
			Self::ChannelCountRead => write!(
				f,
				"no fixed channel count was specified, and could not read channel \
				count for current frame"
			),
		}
	}
}

impl Error {
	fn new(message: Option<String>, kind: ErrorKind, source: Option<Box<dyn error::Error>>) -> Self {
		Self {
			message,
			kind,
			source,
		}
	}

	fn add_msg<M: ToString>(mut self, message: M) -> Self {
		let _ = self.message.insert(message.to_string());
		self
	}

	fn add_write_msg(mut self, data_type: &str, data_field: &str) -> Error {
		self.add_msg(format!("could not write {data_type} ({data_field})"))
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		if let Some(ref message) = self.message {
			write!(f, "{message}: {}", self.kind)
		} else {
			write!(f, "{}", self.kind)
		}
	}
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		self.source.as_deref()
	}
}

impl From<io::Error> for Error {
	fn from(value: io::Error) -> Self {
		Self::new(None, ErrorKind::IO, Some(value.into()))
	}
}

// Data

#[derive(Copy, Clone)]
struct QoaFileHeader {
	magic: [u8; 4],
	sample_count: u32,
}

impl QoaFileHeader {
	const MAGIC: [u8; 4] = *b"qoaf";

	fn new(sample_count: u32) -> Self {
		Self {
			magic: Self::MAGIC,
			sample_count,
		}
	}

	fn check_magic(&self) -> Result {
		if self.magic == Self::MAGIC {
			Ok(())
		} else {
			Err(Error::new(None, ErrorKind::UnknownMagic(self.magic), None))
		}
	}
}

#[allow(non_camel_case_types)]
type u24 = [u8; 3];

#[derive(Copy, Clone)]
struct QoaFrameHeader {
	channel_count: u8,
	sample_rate: u32,
	sample_count: u16,
	size: u16,
}

#[derive(Copy, Clone)]
struct QoaLmsState {
	history: [i16; 4],
	weights: [i16; 4],
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct QoaSlice {
	quant: u8,
	data: [u8; 20],
}

impl QoaSlice {
	const QMASK: u8 = 0xF;
	const DMASK: u8 = 0b111;

	pub fn pack(&self) -> u64 {
		let Self { quant, data } = self;

		let mut value = (*quant as u64) << 60;
		for i in 0..20 {
			value |= ((data[i] & Self::DMASK) as u64) << (19 - i) * 3;
		}
		value
	}

	pub fn unpack(&mut self, value: [u8; 8]) {
		let Self { quant, data } = self;

		let value = u64::from_be_bytes(value);

		*quant = (value >> 60) as u8 & Self::QMASK;
		for i in 0..20 {
			data[i] = (value >> (19 - i) * 3) as u8 & Self::DMASK;
		}
	}
}

#[derive(Copy, Clone)]
struct QoaChannelFrame {
	lms_state: QoaLmsState,
	slices: [QoaSlice; 256],
}

#[derive(Clone)]
struct QoaFrame {
	header: QoaFrameHeader,
	channels: Vec<QoaChannelFrame>
}

#[derive(Copy, Clone)]
enum ModeOptions {
	Streaming,
	Fixed(FixedOptions),
}

#[derive(Copy, Clone, Default)]
struct FixedOptions {
	sample_count: usize,
	sample_rate: usize,
	channel_count: u8,
}

pub struct QoaEncoder<S: Pcm16Source> {
	options: ModeOptions,
	source: S,
}

impl<S: Pcm16Source> QoaEncoder<S> {
	pub fn new_streaming(source: S) -> Self {
		Self {
			options: ModeOptions::Streaming,
			source
		}
	}

	/// Note: `sample_rate` cuts off the last 8 bits (u24).
	pub fn new(
		sample_count: usize,
		sample_rate: usize,
		channel_count: u8,
		source: S
	) -> Self {
		let sample_rate = sample_rate & 0xFFF;
		Self {
			options: ModeOptions::Fixed(
				FixedOptions {
					sample_count,
					sample_rate,
					channel_count,
				}
			),
			source,
		}
	}

	pub fn encode(&mut self, sink: &mut impl Write) -> Result {
		let Self { options, source } = self;

		if let ModeOptions::Fixed(
			FixedOptions { sample_count, sample_rate, channel_count }
		) = options {
			enc(source, *channel_count, *sample_rate, *sample_count, sink)
		} else {
			todo!("Streaming mode is not yet supported");
		}
	}
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
	fn write_int<T: Int>(&mut self, value: T) -> Result where [u8; T::SIZE]: {
		self.write_bytes(&value.into_bytes())
	}

	fn write_byte(&mut self, value: u8) -> Result { self.write_bytes(&[value]) }

	fn write_bytes(&mut self, value: &[u8]) -> Result {
		self.write_all(value).map_err(Error::from)
	}

	fn write_file_header(&mut self, sample_count: u32) -> Result {
		self.write_int(MAGIC)
			.map_err(|err| err.add_write_msg("file header", "magic bytes"))?;
		self.write_int(sample_count)
			.map_err(|err| err.add_write_msg("file header", "sample count"))?;
		Ok(())
	}

	fn write_frame_header(&mut self, channel_count: u8, sample_rate: u32, sample_count: u16, size: u16) -> Result {
		self.write_byte(channel_count)
			.map_err(|err| err.add_write_msg("frame header", "channel count"))?;
		self.write_bytes(&sample_rate.into_bytes()[1..]) // Clip to 24-bits
			.map_err(|err| err.add_write_msg("frame header", "sample rate"))?;
		self.write_int(sample_count)
			.map_err(|err| err.add_write_msg("frame header", "sample count"))?;
		self.write_int(size)
			.map_err(|err| err.add_write_msg("frame header", "size"))?;
		Ok(())
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

	fn enc(self, sink: &mut impl QoaSink) -> Result {
		let Self { history, weights } = self;

		fn pack(acc: u64, cur: i16) -> u64 {
			(acc << 16) | (cur as u16 & 0xFFFF) as u64
		}

		let history = history.into_iter().fold(0, pack);
		let weights = weights.into_iter().fold(0, pack);
		sink.write_int(history)
			.map_err(|err| err.add_write_msg("lms state", "history"))?;
		sink.write_int(weights)
			.map_err(|err| err.add_write_msg("lms state", "weights"))?;
		Ok(())
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

fn enc_frame(
	samples: &[i16],
	sample_rate: u32,
	lms_states: &mut Vec<QoaLmsState>,
	sink: &mut impl QoaSink,
) -> Result {
	let channel_count = lms_states.len() as u8;
	let sample_count = samples.len() as u16;
	let slices = (sample_count + SLICE_LEN as u16 - 1) / SLICE_LEN as u16;
	let size = 24 * channel_count as u16 + 8 * slices * channel_count as u16;

	sink.write_frame_header(channel_count, sample_rate, sample_count, size)?;
	for lms in lms_states.iter() { lms.enc(sink)? }

	for sample in (0..sample_count as usize).step_by(SLICE_LEN) {
		for chn in 0..channel_count {
			let slice_len = SLICE_LEN.clamp(0, sample_count as usize - sample);
			let slice_range = {
				let slice_start = sample * channel_count as usize + chn as usize;
				let slice_end = (sample + slice_len) * channel_count as usize + chn as usize;

				slice_start..slice_end
			};

			let mut best_error = -1;
			let mut best_slice = 0;
			let mut best_lms = QoaLmsState::default();

			for sf in 0..16 {
				let mut lms = lms_states[chn as usize];
				let mut slice = sf as u64;
				let mut cur_err = 0;

				for si in slice_range.clone().step_by(channel_count as usize) {
					let sample = samples[si];
					let predicted = lms.predict();
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

					lms.update(reconst, dequantized);
					slice = slice << 3 | quantized as u64;
				}

				if cur_err < best_error {
					best_error = cur_err;
					best_slice = slice;
					best_lms = lms;
				}
			}

			lms_states[chn as usize] = best_lms;

			best_slice <<= (SLICE_LEN - slice_len) * 3;
			sink.write_int(best_slice)
				.map_err(|err|
					err.add_msg(
						format!("could not write slice on channel {chn}")
					)
				)?;
		}
	}
	Ok(())
}

fn enc(
	samples: &mut impl Pcm16Source,
	channel_count: u8,
	sample_rate: usize,
	sample_count: usize,
	sink: &mut impl Write
) -> Result {
	if !(1..=16777215).contains(&sample_rate) {
		return Err(Error::new(None, ErrorKind::InvalidRate(sample_rate), None));
	}

	let mut lms = {
		let mut channels = Vec::with_capacity(channel_count as usize);
		channels.fill(QoaLmsState::default());
		channels
	};

	sink.write_file_header(sample_count as u32)?;

	// Streaming mode
	let sample_count = if sample_count == 0 {
		usize::MAX
	} else {
		sample_count
	};
	let var_channels = channel_count == 0;

	let mut frame_buf = [0; FRAME_LEN];
	while !samples.exhausted() {
		let channel_count = if var_channels {
			samples.channels()
				.map_err(|err|
					Error::new(None, ErrorKind::ChannelCountRead, Some(err.into()))
				)?
		} else {
			channel_count
		};

		let mut frame_len = FRAME_LEN.clamp(0, sample_count) * channel_count as usize;
		frame_len = samples.read_frame(&mut frame_buf[..frame_len])
						   .map_err(|err|
							   Error::new(
								   None,
								   ErrorKind::SampleRead,
								   Some(err.into())
							   )
						   )?;
		enc_frame(&frame_buf[..frame_len], sample_rate as u32, &mut lms, sink)?;
	}
	Ok(())
}

// IO

/// A raw PCM-S16LE audio source.
pub trait Pcm16Source {
	type Error: error::Error + Into<Box<dyn error::Error>>;

	/// Returns true if the source has no more samples.
	fn exhausted(&mut self) -> Result<bool, Self::Error>;

	/// Returns the number of channels the source has. Called before
	/// [`Self::read_frame`] to size the sample buffer in streaming mode.
	fn channels(&mut self) -> Result<u8, Self::Error>;

	/// Reads a frame to `buf`, returning the number of samples read.
	fn read_frame(&mut self, buf: &mut [i16]) -> Result<usize, Self::Error>;
}

pub struct Pcm16Reader<R: Read> {
	channel_count: u8,
	source: BufReader<R>,
}

impl<R: Read> Pcm16Reader<R> {
	pub fn new(channel_count: u8, source: BufReader<R>) -> Self {
		assert!(channel_count > 0);
		Self {
			channel_count,
			source,
		}
	}
}

impl<R: Read> Pcm16Source for Pcm16Reader<R> {
	type Error = Error;

	fn exhausted(&mut self) -> Result<bool> {
		Ok(!self.source.has_data_left().map_err(Error::from)?)
	}

	fn channels(&mut self) -> Result<u8> {
		Ok(self.channel_count)
	}

	fn read_frame(&mut self, buf: &mut [i16]) -> Result<usize> {
		const N: usize = mem::size_of::<i16>();
		let mut bytes = Vec::with_capacity(buf.len() * N);
		let count = self.source.read(&mut bytes).map_err(Error::from)?;
		let samples: Vec<_> = bytes.array_windows::<N>()
								   .take(count / N)
								   .map(|sample| u16::from_be_bytes(*sample) as i16)
								   .collect();
		buf.copy_from_slice(&samples);
		Ok(samples.len())
	}
}
