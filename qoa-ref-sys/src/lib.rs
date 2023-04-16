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

//! A wrapper around the C reference codec, found at: https://github.com/phoboslab/qoa/.
//! Used for testing and benchmarking QOAR.

pub mod qoa;

use std::error::Error;
use std::path::PathBuf;
use std::ptr::slice_from_raw_parts_mut;
use std::ffi::CString;
use qoa::*;
pub use qoa::qoa_desc as QoaDesc;

impl Default for QoaDesc {
	fn default() -> Self {
		Self {
			channels: 0,
			samplerate: 0,
			samples: 0,
			lms: [qoa_lms_t::default(); 8],
			error: 0.0,
		}
	}
}

impl Default for qoa_lms_t {
	fn default() -> Self {
		Self {
			history: [0; 4],
			weights: [0, 0, -(1 << 13), 1 << 14],
		}
	}
}

pub fn encode(source: &[i16], descriptor: &mut QoaDesc) -> Result<Box<[u8]>, &'static str> {
	let ref mut len = 0;

	Ok(unsafe {
		let data = slice_from_raw_parts_mut(
			qoa_encode(source.as_ptr(), descriptor, len).cast(),
			*len as usize
		);

		if data.is_null() {
			return Err("encode error")
		}

		Box::from_raw(data)
	})
}

pub fn read_wav(path: PathBuf, descriptor: &mut QoaDesc) -> Result<Box<[i16]>, Box<dyn Error>> {
	let path = CString::new(path.to_str().ok_or("invalid path")?)?;

	Ok(unsafe {
		let data = slice_from_raw_parts_mut(
			qoaconv_wav_read(path.as_ptr(), descriptor),
			descriptor.samples as usize
		);

		Box::from_raw(data)
	})
}

pub fn decode(source: &[u8], descriptor: &mut QoaDesc) -> Result<Box<[i16]>, &'static str> {
	Ok(unsafe {
		let data = slice_from_raw_parts_mut(
			qoa_decode(source.as_ptr(), source.len() as i32, descriptor).cast(),
			descriptor.samples as usize
		);

		if data.is_null() {
			return Err("decode error")
		}

		Box::from_raw(data)
	})
}

impl qoa_lms_t {
	pub fn predict(&mut self) -> i32 {
		unsafe { qoa_lms_predict(self) }
	}

	pub fn update(&mut self, sample: i16, residual: i32) {
		unsafe { qoa_lms_update(self, sample as i32, residual) }
	}
}
