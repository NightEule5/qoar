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
use std::{fmt, io};
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

pub const TEST_SAMPLE_URL: &str = "https://qoaformat.org/samples/qoa_test_samples_2023_02_18.zip";
pub const TEST_SAMPLE_DIR: &str = "run/qoa_test_samples_2023_02_18";
pub const TEST_SAMPLE_ZIP: &str = "qoar/qoa_test_samples_2023_02_18.zip";

fn sample_path(group: &str, base: Option<&str>, sample: &str, ext: &str) -> PathBuf {
	let mut path: PathBuf = TEST_SAMPLE_DIR.into();
	path.push(group);
	path.extend(base);
	path.push(format!("{sample}.{ext}"));
	path
}

macro_rules! samples {
    ($($enum_name:ident=>$group:literal{$($sample:ident=>$name:literal)+})+) => {
		$(
		pub enum $enum_name {
			$($sample),+
		}

		impl $enum_name {
			const SAMPLES: &[Self] = &[ $(Self::$sample),+ ];
		}

		impl Sample for $enum_name {
			fn name(&self) -> &str {
				match self {
					$(Self::$sample => $name),+
				}
			}

			fn group(&self) -> &str { $group }
		}
		)+
	};
}

samples! {
	BandcampSample => "bandcamp" {
		AllegaeonBeastsAndWorms				=> "allegaeon-beasts-and-worms"
		DarksideNarrowRoad					=> "darkside_narrow_road"
		ForestSwordsAnnekasBattle			=> "forest_swords_annekas_battle"
		FourTetBaby							=> "four_tet_baby"
		InversePhaseAtaribleLie				=> "inverse_phase_atarible_lie"
		JulienBakerSprainedAnkle			=> "julien_baker_sprained_ankle"
		LornDrawnOutLikeAnAche				=> "lorn_drawn_out_like_an_ache"
		NilsFrahmAllMelody					=> "nils_frahm_all_melody"
		PlanesMistakenForStarsOneFuckedPony	=> "planes_mistaken_for_stars_one_fucked_pony"
		WhaleriderDevilGotMe				=> "whalerider_devil_got_me"
	}
	
	OculusAudioPack => "oculus_audio_pack" {
		ActionDropCoin					=> "action_drop_coin_01"
		ActionDropPaperBall				=> "action_drop_paper_ball"
		ActionShellCasingOnGrass		=> "action_shell_casing_on_grass_01"
		ActionSwordScrape				=> "action_sword_scrape_02"
		ActionTyping					=> "action_typing"
		AmbienceForestBirds1			=> "ambience_forest_birds_02"
		AmbienceForestBirds2			=> "ambience_forest_birds_03"
		AmbientCityRainLp				=> "ambient_city_rain_lp"
		AmbientFarmGenericLp			=> "ambient_farm_generic_lp"
		AmbientNightLp					=> "ambient_night_lp"
		AmbientWaterLakeShoreLp		=> "ambient_water_lake_shore_01_lp"
		AmbientWindChimesLp				=> "ambient_wind_chimes_lp_01"
		AmbientWoodCutAxe				=> "ambient_wood_cut_axe"
		BodyMovementClothingGeneric		=> "body_movement_clothing_generic_04"
		BodyMovementWithGear			=> "body_movement_withGear_03"
		BounceCartoony					=> "bounce_cartoony_03"
		CarEngineHighLp					=> "car_engine_high_lp"
		CarStartRunStop					=> "car_start_run_stop_01"
		CarTrunkClose					=> "car_trunk_close"
		CreepyBloodSquishSlimy			=> "creepy_blood_squish_slimy_03"
		CreepyChains					=> "creepy_chains_02"
		CreepyCreature					=> "creepy_creature_03"
		CreepyDrone						=> "creepy_drone_02"
		CreepyImpactsReverb				=> "creepy_impacts_reverb_01"
		CreepyScratch					=> "creepy_scratch_02"
		CreepySkullCracking				=> "creepy_skull_cracking_02"
		CreepyWhispers					=> "creepy_whispers_01"
		DoorLockKey						=> "door_lock_key_03"
		DoorsFrontdoorOpen				=> "doors_frontdoor_open_01"
		DoorsOfficeDoorknob				=> "doors_office_doorknob_01"
		DoorsSlidingLock				=> "doors_sliding_lock_01"
		FootstepsShoeConcreteRun		=> "footsteps_shoe_concrete_run_04"
		FootstepsShoeDirtRun			=> "footsteps_shoe_dirt_run_03"
		FootstepsShoeGrassRun			=> "footsteps_shoe_grass_run_02"
		FootstepsShoeGrassWalk			=> "footsteps_shoe_grass_walk_05"
		FootstepsShoeMetalRun			=> "footsteps_shoe_metal_run_04"
		FootstepsShoeSnowWalk			=> "footsteps_shoe_snow_walk_03"
		IndoorDrawerClose				=> "Indoor_drawer_close_02"
		IndoorFanOff					=> "Indoor_fan_off"
		IndoorHydraulic					=> "indoor_hydraulic_01"
		IndoorLeverPull					=> "indoor_lever_pull_03"
		IndoorSwitchSmallOn				=> "Indoor_switch_small_on_01"
		InteractionBookPageTurns		=> "interaction_book_page_turns"
		InteractionFaucetOn				=> "interaction_faucet_on"
		InteractionKnapsackNylonClose	=> "interaction_knapsack_nylon_close"
		InteractionMagicSpell			=> "interaction_magic_spell_01"
		InteractionValveTurn			=> "interaction_valve_01_turn"
		InteractionWhooshMedium			=> "interaction_whoosh_medium_02"
		MachinePowerTool				=> "machine_power_tool_04"
		StingAcousticGuitPos			=> "sting_acoustic_guit_pos_01"
		StingBanjoHumorous				=> "sting_banjo_humorous_02"
		StingLossMallet					=> "sting_loss_mallet"
		StingLossPiano					=> "sting_loss_piano"
		StingVictoryMallet				=> "sting_victory_mallet"
		StingVictoryOrch				=> "sting_victory_orch_03"
		StingXpLevelUpOrch				=> "sting_xp_level_up_orch_01"
		Swoosh							=> "swoosh_03"
		UiCasualMusicalOpen				=> "ui_casual_musical_open"
		UiLaserShoot					=> "ui_laser_shoot_02"
		UiMagicalOpen					=> "ui_magical_open"
		UiMetalOpen						=> "ui_metal_open"
		UiNotification					=> "ui_notification_03"
		UiPowerup						=> "ui_powerup_01"
		UiScifiTraditionalConfirm		=> "ui_scifi_traditional_confirm"
		UiWoodError						=> "ui_wood_error"
		VoiceAnimalDuck					=> "voice_animal_duck_01"
		VoiceAnimalSheep				=> "voice_animal_sheep_01"
		VoiceEvilLaugh					=> "voice_evil_laugh_02"
		VoiceHorseNeigh					=> "voice_horse_neigh_01"
		VoiceMaleBreathing				=> "voice_male_breathing_01"
		VoiceMaleScream					=> "voice_male_scream_01"
	}
}

pub trait Sample: Sized {
	fn name(&self) -> &str;
	fn group(&self) -> &str;

	fn wav_path(&self) -> PathBuf {
		sample_path(self.group(), None, self.name(), "wav")
	}

	fn qoa_path(&self) -> PathBuf {
		sample_path(self.group(), Some("qoa"), self.name(), "qoa")
	}

	fn dec_path(&self) -> PathBuf {
		sample_path(self.group(), Some("qoa_wav"), self.name(), "qoa.wav")
	}

	fn decode_wav(&self) -> Result<FormatSource, Box<dyn Error>> {
		decode_wav(self.wav_path())
	}
}

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
pub struct DisplayError(pub Box<dyn Error>);

impl Debug for DisplayError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
pub struct OpaqueData<'a, T: Eq + PartialEq + 'a>(pub &'a [T]);

impl<T: Eq + PartialEq> Deref for OpaqueData<'_, T> {
	type Target = [T];

	fn deref(&self) -> &Self::Target { self.0 }
}

impl<T: Debug + Eq + PartialEq> Debug for OpaqueData<'_, T> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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

impl<'a, T: Eq + PartialEq + 'a> From<&'a [T]> for OpaqueData<'a, T> {
	fn from(value: &'a [T]) -> Self { Self(value) }
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
