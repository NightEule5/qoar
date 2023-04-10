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

use std::env::temp_dir;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::fs::{create_dir_all, File, remove_dir, remove_file};
use std::io;
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use amplify_derive::Display;
use ctor::ctor;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::get;
use symphonia::core::codecs::{CODEC_TYPE_PCM_S16LE, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::{Hint, ProbeResult};
use symphonia::default::{get_codecs, get_probe};
use zip::ZipArchive;
use qoar::conv::FormatSource;

pub const TEST_SAMPLE_URL: &'static str = "https://qoaformat.org/samples/qoa_test_samples_2023_02_18.zip";
pub const TEST_SAMPLE_DIR: &'static str = "run/qoa_test_samples_2023_02_18";
pub const TEST_SAMPLE_ZIP: &'static str = "qoar/qoa_test_samples_2023_02_18.zip";

// https://gist.github.com/giuliano-oliveira/4d11d6b3bb003dba3a1b53f43d81b30d
#[ctor]
fn download_test_samples() {
	if Path::new(TEST_SAMPLE_DIR).exists() { return }

	let zip = temp_dir().join(TEST_SAMPLE_ZIP);
	create_dir_all(zip.parent().unwrap()).unwrap();

	struct ProgressWriter {
		file: File,
		progress: ProgressBar,
		downloaded: u64
	}

	impl Write for ProgressWriter {
		fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
			let written = self.file.write(buf)?;
			self.downloaded += written as u64;
			self.progress.set_position(self.downloaded);
			Ok(written)
		}

		fn flush(&mut self) -> io::Result<()> {
			self.file.flush()
		}
	}

	if !zip.exists() {
		println!("Downloading {TEST_SAMPLE_URL}");
		let mut response = get(TEST_SAMPLE_URL).unwrap();
		let total_size = response.content_length().unwrap();

		let progress = ProgressBar::new(total_size);
		progress.set_style(
			ProgressStyle::default_bar()
				.template(
					"{msg} -> [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}\
					/{total_bytes} ({bytes_per_sec}, {eta})"
				).unwrap()
				.progress_chars("#>-")
		);
		progress.set_message(TEST_SAMPLE_ZIP);

		let mut writer = ProgressWriter {
			file: File::create(&zip).unwrap(),
			progress,
			downloaded: 0,
		};
		response.copy_to(&mut writer).unwrap();

		writer.progress.finish();
		println!("Downloaded {TEST_SAMPLE_URL} to {}", zip.display());
	}

	println!("Extracting {TEST_SAMPLE_ZIP}");
	{
		let file = File::open(&zip).unwrap();
		let mut zip = ZipArchive::new(file).unwrap();

		for i in 0..zip.len() {
			let mut file = zip.by_index(i).unwrap();
			let path = Path::new(TEST_SAMPLE_DIR)
				.join(file.enclosed_name().expect("invalid path"));

			if file.name().ends_with('/') {
				create_dir_all(path).unwrap();
			} else {
				if let Some(parent) = path.parent() {
					create_dir_all(parent).unwrap();
				}

				let mut out = File::create(path).unwrap();
				io::copy(&mut file, &mut out).unwrap();
			}
		}
	}
	println!("Extracted {TEST_SAMPLE_ZIP} to {TEST_SAMPLE_DIR}");

	remove_file(&zip).unwrap();
	remove_dir(zip.parent().unwrap()).unwrap();
}

/// A workaround for errors using Debug instead of Display.
#[derive(Display)]
#[display("{0}")]
struct DisplayError(Box<dyn Error>);

impl Debug for DisplayError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		Display::fmt(self, f)
	}
}

impl Error for DisplayError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		Some(self.0.as_ref())
	}
}

/// A large block of data that can be asserted without overwhelming the output.
#[derive(Eq, PartialEq)]
struct OpaqueData<T: Eq + PartialEq>(Vec<T>);

impl<T: Eq + PartialEq> Deref for OpaqueData<T> {
	type Target = Vec<T>;

	fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T: Debug + Eq + PartialEq> Debug for OpaqueData<T> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let len = self.len();
		if len > 100 {
			f.debug_list()
				.entries(self.iter().take(25))
				.entry(&format!("[{} items...]", len - 50))
				.entries(self.iter().skip(len - 25))
				.finish()
		} else {
			self.deref().fmt(f)
		}
	}
}

impl<T: Eq + PartialEq> From<Vec<T>> for OpaqueData<T> {
	fn from(value: Vec<T>) -> Self { Self(value) }
}

fn decode_wav(file_name: PathBuf) -> Result<FormatSource, Box<dyn Error>> {
	let file = File::open(file_name)?;
	let registry = get_codecs();
	let probe = get_probe();
	let source = MediaSourceStream::new(
		Box::new(file),
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
					   .ok_or("unsupported codec")?;
	let decoder = registry.make(&track.codec_params, &DecoderOptions::default())?;
	Ok(FormatSource::new(track.clone(), demuxer, decoder))
}
