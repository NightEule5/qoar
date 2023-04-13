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
use qoa_ref_sys::{encode, QoaDesc, read_wav};
use crate::OculusAudioPack::*;
use crate::common::{Sample, OpaqueData};

macro_rules! gen_encode {
    ($($name:ident($sample:ident))+) => {
		$(
		#[bench]
		fn $name(b: &mut Bencher) -> Result<(), Box<dyn Error>> {
			let ref mut descriptor = QoaDesc::default();
			let data = read_wav($sample.wav_path(), descriptor)?;

			b.iter(|| encode(data, descriptor));
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
