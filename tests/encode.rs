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

include!("common.rs");

use std::fs::read;
use qoar::{Encoder, PcmSource, PcmStream};
use qoar::io::Buffer;

#[test]
fn encode_oculus_audio_pack() {
	encode_sample("oculus_audio_pack", "action_drop_coin_01")
		.map_err(DisplayError)
		.unwrap()
}

fn encode_sample(group: &str, name: &str) -> Result<(), Box<dyn Error>> {
	const PREFIX: &str = "run/qoa_test_samples_2023_02_18";
	let qoa_path: PathBuf = format!("{PREFIX}/{group}/qoa/{name}.qoa").into();
	let wav_path: PathBuf = format!("{PREFIX}/{group}/{name}.wav").into();

	let mut wav = decode_wav(wav_path)?;
	let samples  = wav.sample_count(0) as u32;
	let rate     = wav.sample_rate();
	let channels = wav.channel_count();

	let mut enc = Encoder::new_fixed(samples, rate, channels, Buffer::default())?;
	enc.encode(&mut wav)?;
	let enc = enc.close().unwrap()?.encode();
	let qoa = read(qoa_path)?;

	assert_eq!(OpaqueData(enc), OpaqueData(qoa));

	Ok(())
}