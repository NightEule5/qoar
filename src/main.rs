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

#![feature(assert_matches)]

use std::assert_matches::assert_matches;
use std::env::args;
use std::error::Error as StdError;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use amplify_derive::{Display, Error as AmpError};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::{Hint, ProbeResult};
use symphonia::default::{get_codecs, get_probe};
use qoar::conv::FormatSource;
use qoar::Encoder;

#[derive(Clone, Debug, Display, AmpError)]
enum Error {
	#[display("missing {0} argument")]
	MissingArguments(MissingArgument),
	#[display("unknown command {0}")]
	UnknownCommand(String),
	#[display("no tracks found")]
	NoTracks,
}

#[derive(Copy, Clone, Debug, Display)]
enum MissingArgument {
	#[display("command")]
	Command,
	#[display("source file")]
	SourceFile,
	#[display("destination file")]
	DestinationFile,
}

fn main() { run(args().skip(1)).unwrap() }

fn run(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn StdError>> {
	let cmd = args.next().ok_or(Error::MissingArguments(MissingArgument::Command))?;
	let src = args.next().ok_or(Error::MissingArguments(MissingArgument::SourceFile))?.into();
	let dst = args.next().ok_or(Error::MissingArguments(MissingArgument::DestinationFile))?.into();

	if cmd == "encode" {
		enc(src, dst)
	} else {
		Err(Error::UnknownCommand(cmd).into())
	}
}

fn enc(src: PathBuf, dst: PathBuf) -> Result<(), Box<dyn StdError>> {
	assert_matches!(
		dst.extension()
		   .map(|ext| ext.to_string_lossy())
		   .as_deref(),
		Some("qoa")
	);

	let src = File::open(src)?;
	let dst = File::options().truncate(true)
							 .create(true)
							 .write(true)
							 .open(dst)?;
	let registry = get_codecs();
	let probe = get_probe();
	let source = MediaSourceStream::new(
		Box::new(src),
		MediaSourceStreamOptions::default()
	);
	let ProbeResult { format: demuxer, .. } = probe.format(
		&Hint::new(),
		source,
		&FormatOptions::default(),
		&MetadataOptions::default()
	)?;
	let track = demuxer.default_track().ok_or(Error::NoTracks)?.clone();
	let decoder = registry.make(&track.codec_params, &DecoderOptions::default())?;

	let mut source = FormatSource::new(track.clone(), demuxer, decoder);

	let mut enc = Encoder::new_fixed(
		track.codec_params.n_frames.unwrap_or_default() as u32,
		track.codec_params.sample_rate.unwrap_or_default(),
		track.codec_params.channels.map(Channels::count).unwrap_or_default() as u8,
		BufWriter::new(dst),
	)?;
	enc.encode(&mut source)?;
	Ok(())
}
