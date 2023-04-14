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

mod common;

use std::error::Error;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::fs::read;
use qoa_ref_sys::{decode, QoaDesc};
use qoar::byte_decoder::Decoder;
use qoar::io::Buffer;
use qoar::{PcmBuffer, PcmFrame, PcmSink};
use crate::common::{DisplayError, OculusAudioPack, OpaqueData, Sample};

#[test]
fn decode_oculus_audio_pack() -> Result<(), DisplayError> {
	decode_sample(OculusAudioPack::ActionDropCoin01)
		.map_err(DisplayError)
}

fn decode_sample(sample: impl Sample) -> Result<(), Box<dyn Error>> {
	let data = read(sample.qoa_path())?;
	let dec = {
		let mut buf = Vec::new();
		Decoder::default().decode(&*data, &mut buf)?;
		buf
	};
	let qoa = decode(&*data, &mut QoaDesc::default())?;;

	assert_eq!(OpaqueData(&dec), OpaqueData(qoa));

	Ok(())
}

#[derive(Eq, PartialEq)]
struct OpaqueFrame(PcmFrame);

impl Debug for OpaqueFrame {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let Self(frame) = self;

		f.debug_struct("Frame")
			.field("rate", &frame.rate())
			.field("channels", &frame.channels())
			.field("data", &OpaqueData(frame.data()))
			.finish()
	}
}
