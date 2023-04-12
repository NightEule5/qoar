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
use std::fs::File;
use std::path::PathBuf;
use qoar::{Decoder, PcmBuffer, PcmSource};
use crate::common::{decode_wav, DisplayError, OpaqueData};

#[test]
fn decode_oculus_audio_pack() {
	decode_sample("oculus_audio_pack", "action_drop_coin_01")
		.map_err(DisplayError)
		.unwrap()
}

fn decode_sample(group: &str, name: &str) -> Result<(), Box<dyn Error>> {
	const PREFIX: &str = "run/qoa_test_samples_2023_02_18";
	let qoa_path: PathBuf = format!("{PREFIX}/{group}/qoa-ref/{name}.qoa-ref").into();
	let wav_path: PathBuf = format!("{PREFIX}/{group}/qoa_wav/{name}.qoa-ref.wav").into();

	let qoa = Decoder::new(PcmBuffer::default())
		.decode(&mut File::open(qoa_path)?)?
		.unwrap();
	let wav = decode_wav(wav_path)?.read_all()?;

	assert_eq!(OpaqueData(qoa), OpaqueData(wav));

	Ok(())
}
