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

macro_rules! gen {
	($($name:ident($sample:ident))+) => {
		mod encode {
			use crate::{OculusAudioPack::*, Sample};
			use test::Bencher;
			use qoa_ref_sys::{encode, QoaDesc, read_wav};
			use std::error::Error;

			$(
			#[bench]
			fn $name(b: &mut Bencher) -> Result<(), Box<dyn Error>> {
				let ref mut descriptor = QoaDesc::default();
				let ref data = read_wav($sample.wav_path(), descriptor)?;

				b.iter(|| encode(data, descriptor));
				Ok(())
			}
			)+
		}

		mod decode {
			use crate::{OculusAudioPack::*, Sample};
			use test::Bencher;
			use qoa_ref_sys::{decode, QoaDesc};
			use std::fs::read;
			use std::error::Error;

			$(
			#[bench]
			fn $name(b: &mut Bencher) -> Result<(), Box<dyn Error>> {
				let mut desc = QoaDesc::default();
				let data = read($sample.qoa_path())?;
				b.iter(|| decode(&data, &mut desc));
				Ok(())
			}
			)+
		}
	};
}

gen! {
	action_drop_coin(ActionDropCoin)
	action_drop_paper_ball(ActionDropPaperBall)
	action_shell_casing_on_grass(ActionShellCasingOnGrass)
	action_sword_scrape(ActionSwordScrape)
	action_typing(ActionTyping)
	ambience_forest_birds1(AmbienceForestBirds1)
	ambience_forest_birds2(AmbienceForestBirds2)
	ambient_city_rain_lp(AmbientCityRainLp)
	ambient_farm_generic_lp(AmbientFarmGenericLp)
	ambient_night_lp(AmbientNightLp)
	ambient_water_lake_shore_lp(AmbientWaterLakeShoreLp)
	ambient_wind_chimes_lp(AmbientWindChimesLp)
	ambient_wood_cut_axe(AmbientWoodCutAxe)
	body_movement_clothing_generic(BodyMovementClothingGeneric)
	body_movement_with_gear(BodyMovementWithGear)
	bounce_cartoony(BounceCartoony)
	car_engine_high_lp(CarEngineHighLp)
	car_start_run_stop(CarStartRunStop)
	car_trunk_close(CarTrunkClose)
	creepy_blood_squish_slimy(CreepyBloodSquishSlimy)
	creepy_chains(CreepyChains)
	creepy_creature(CreepyCreature)
	creepy_drone(CreepyDrone)
	creepy_impacts_reverb(CreepyImpactsReverb)
	creepy_scratch(CreepyScratch)
	creepy_skull_cracking(CreepySkullCracking)
	creepy_whispers(CreepyWhispers)
	door_lock_key(DoorLockKey)
	doors_frontdoor_open(DoorsFrontdoorOpen)
	doors_office_doorknob(DoorsOfficeDoorknob)
	doors_sliding_lock(DoorsSlidingLock)
	footsteps_shoe_concrete_run(FootstepsShoeConcreteRun)
	footsteps_shoe_dirt_run(FootstepsShoeDirtRun)
	footsteps_shoe_grass_run(FootstepsShoeGrassRun)
	footsteps_shoe_grass_walk(FootstepsShoeGrassWalk)
	footsteps_shoe_metal_run(FootstepsShoeMetalRun)
	footsteps_shoe_snow_walk(FootstepsShoeSnowWalk)
	indoor_drawer_close(IndoorDrawerClose)
	indoor_fan_off(IndoorFanOff)
	indoor_hydraulic(IndoorHydraulic)
	indoor_lever_pull(IndoorLeverPull)
	indoor_switch_small_on(IndoorSwitchSmallOn)
	interaction_book_page_turns(InteractionBookPageTurns)
	interaction_faucet_on(InteractionFaucetOn)
	interaction_knapsack_nylon_close(InteractionKnapsackNylonClose)
	interaction_magic_spell(InteractionMagicSpell)
	interaction_valve_turn(InteractionValveTurn)
	interaction_whoosh_medium(InteractionWhooshMedium)
	machine_power_tool(MachinePowerTool)
	sting_acoustic_guit_pos(StingAcousticGuitPos)
	sting_banjo_humorous(StingBanjoHumorous)
	sting_loss_mallet(StingLossMallet)
	sting_loss_piano(StingLossPiano)
	sting_victory_mallet(StingVictoryMallet)
	sting_victory_orch(StingVictoryOrch)
	sting_xp_level_up_orch(StingXpLevelUpOrch)
	swoosh(Swoosh)
	ui_casual_musical_open(UiCasualMusicalOpen)
	ui_laser_shoot(UiLaserShoot)
	ui_magical_open(UiMagicalOpen)
	ui_metal_open(UiMetalOpen)
	ui_notification(UiNotification)
	ui_powerup(UiPowerup)
	ui_scifi_traditional_confirm(UiScifiTraditionalConfirm)
	ui_wood_error(UiWoodError)
	voice_animal_duck(VoiceAnimalDuck)
	voice_animal_sheep(VoiceAnimalSheep)
	voice_evil_laugh(VoiceEvilLaugh)
	voice_horse_neigh(VoiceHorseNeigh)
	voice_male_breathing(VoiceMaleBreathing)
	voice_male_scream(VoiceMaleScream)
}
