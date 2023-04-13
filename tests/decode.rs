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
use std::fs::read;
use qoa_ref_sys::{decode, QoaDesc};
use qoar::{Decoder, PcmBuffer};
use qoar::io::Buffer;
use crate::common::{DisplayError, OculusAudioPack, OpaqueData, Sample};

#[test]
fn decode_oculus_audio_pack() -> Result<(), DisplayError> {
	decode_sample(OculusAudioPack::ActionDropCoin01)
		.map_err(DisplayError)
}

fn decode_sample(sample: impl Sample) -> Result<(), Box<dyn Error>> {
	let data = read(sample.qoa_path())?;
	let dec = Decoder::new(PcmBuffer::default())
		.decode(&mut Buffer::decode(&mut data.clone()))?
		.unwrap();
	let qoa = decode(&*data, &mut QoaDesc::default())?.to_vec();

	assert_eq!(OpaqueData(dec), OpaqueData(qoa));

	Ok(())
}
