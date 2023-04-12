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

use std::error::Error;
use test::Bencher;
use qoar::{Encoder, PcmSource, PcmStream, StreamDescriptor};
use qoar::io::Buffer;
use crate::OculusAudioPack::*;
use crate::Sample;

macro_rules! gen_encode {
    ($($name:ident($sample:ident))+) => {
		$(
		#[bench]
		fn $name(b: &mut Bencher) -> Result<(), Box<dyn Error>> {
			let mut data = $sample.decode_wav()?;
			let samples  = data.sample_count(0) as u32;
			let channels = data.channel_count();
			let rate     = data.sample_rate();
			let ref data = data.read_all()?;

			b.iter(|| {
				Encoder::new_fixed(samples, rate, channels, Buffer::default())?
					.encode_vec(&mut data.clone(), StreamDescriptor::default())
			});
			Ok(())
		}
		)+
	};
}

gen_encode! {
	encode_action_drop_coin(ActionDropCoin01)
	encode_action_drop_paper_ball(ActionDropPaperBall)
	encode_action_shell_casing_on_grass(ActionShellCasingOnGrass01)
	encode_action_sword_scrape(ActionSwordScrape02)
}