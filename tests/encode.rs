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

use std::env;
use std::error::Error;
use std::fs::{File, read};
use std::path::PathBuf;
use sha2::{Digest, Sha256};
use symphonia::core::codecs::{CODEC_TYPE_PCM_S16LE, DecoderOptions};
use symphonia::core::errors;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::{Hint, ProbeResult};
use symphonia::default::{get_codecs, get_probe};
use qoar::conv::FormatSource;
use qoar::{Encoder, Pcm16Source};

#[test]
fn encode() -> Result<(), Box<dyn Error>> {
	let cwd = env::current_dir()?;
	test(cwd.join("run/allegaeon-beasts-and-worms"))
}

fn test(sample_name: PathBuf) -> Result<(), Box<dyn Error>> {
	let wav_src = File::open(sample_name.with_extension("wav"))?;
	let qoa_src = read(sample_name.with_extension("qoa"))?;
	let qoa_enc = {
		let registry = get_codecs();
		let probe = get_probe();
		let source = MediaSourceStream::new(
			Box::new(wav_src),
			MediaSourceStreamOptions::default()
		);
		let ProbeResult { format: demuxer, .. } = probe.format(
			&Hint::new(),
			source,
			&FormatOptions::default(),
			&MetadataOptions::default()
		)?;
		let track = demuxer.tracks()
						   .iter()
						   .find(|track| track.codec_params.codec == CODEC_TYPE_PCM_S16LE)
						   .ok_or(errors::Error::Unsupported("unsupported codec"))?;
		let decoder = registry.make(&track.codec_params, &DecoderOptions::default())?;
		let mut source = FormatSource::new(track.clone(), demuxer, decoder);

		let desc = source.descriptor()?;
		let mut enc = Encoder::new_fixed(
			desc.samples().unwrap_or_default(),
			desc.rate().unwrap_or_default(),
			desc.channels().unwrap_or_default(),
			Vec::new(),
		)?;
		enc.encode(&mut source)?;
		enc.close().ok_or(qoar::Error::Closed)??
	};

	let enc = {
		let ref mut hash = Sha256::default()
			.chain_update(qoa_enc)
			.finalize();
		base16ct::upper::encode_string(hash)
	};

	let src = {
		let ref mut hash = Sha256::default()
			.chain_update(qoa_src)
			.finalize();
		base16ct::upper::encode_string(hash)
	};

	assert_eq!(enc, src);
	Ok(())
}
