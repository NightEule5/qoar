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

#![feature(test)]

extern crate test;

use std::ptr::slice_from_raw_parts;

include!("common.rs");
include!("qoa.rs");

fn qoa_enc(source: &[i16], channels: u32, samplerate: u32, samples: u32) -> Result<&[u8], &str> {
	let ref mut len = 0;
	let ref mut desc = qoa_desc {
		channels,
		samplerate,
		samples,
		lms: [qoa_lms_t {
			history: [0; 4],
			weights: [0; 4],
		}; 8],
	};

	unsafe {
		slice_from_raw_parts(
			qoa_encode(source.as_ptr(), desc, len).cast(),
			*len as usize
		).as_ref().ok_or("encode error")
	}
}

fn qoa_dec(source: &[u8]) -> Result<&[i16], &str> {
	let ref mut desc = qoa_desc {
		channels: 0,
		samplerate: 0,
		samples: 0,
		lms: [qoa_lms_t {
			history: [0; 4],
			weights: [0; 4],
		}; 8],
	};

	unsafe {
		slice_from_raw_parts(
			qoa_decode(source.as_ptr(), source.len() as i32, desc).cast(),
			desc.samples as usize
		).as_ref().ok_or("encode error")
	}
}
