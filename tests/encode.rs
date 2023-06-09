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
use qoa_ref_sys::{encode, QoaDesc, read_wav};
use qoar::{Encoder, PcmSource, PcmStream, StreamDescriptor};
use qoar::io::Buffer;
use crate::common::{DisplayError, OculusAudioPack, OpaqueData, Sample};

#[test]
fn encode_oculus_audio_pack() -> Result<(), DisplayError> {
	encode_sample(OculusAudioPack::ActionDropCoin)
		.map_err(DisplayError)
}

fn encode_sample(sample: impl Sample) -> Result<(), Box<dyn Error>> {
	let wav = sample.decode_wav()?;
	let samples  = wav.sample_count() as u32;
	let rate     = wav.sample_rate();
	let channels = wav.channel_count();
	let mut data = wav.read_all()?;

	let mut enc = Encoder::new_fixed(samples as usize, rate, channels, Buffer::default())?;
	enc.encode(&mut data)?;
	let enc = enc.close().unwrap()?.encode();
	let qoa = {
		let ref mut descriptor = QoaDesc::default();
		let data = read_wav(sample.wav_path(), descriptor)?;

		encode(data.as_ref(), descriptor)?
	};

	assert_eq!(OpaqueData(&*enc), OpaqueData(&*qoa));

	Ok(())
}