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

use crate::{DEQUANT_TABLE, div, QoaLmsState, QUANT_TABLE, SLICE_LEN};
use std::simd::SimdInt;

#[cfg(feature = "simd")]
pub use simd::VectorScaler;

pub trait SliceScaler {
	fn scale(samples: &[i16], lms: &mut QoaLmsState, chn: usize, channel_count: usize) -> u64;
}

/// A linear scaler, the method the reference encoder uses. Computes the error for
/// each scale factor in sequence.
pub struct LinearScaler;

impl LinearScaler {
	fn scale_sample(sample: i32, sf: usize, lms: &mut QoaLmsState) -> (u8, i32, i16) {
		let predicted = lms.predict();
		let residual = sample - predicted;
		let scaled = div(residual, sf);
		let clamped = scaled.clamp(-8, 8);
		let quantized = QUANT_TABLE[(clamped + 8) as usize];
		let dequantized = DEQUANT_TABLE[sf][quantized as usize];
		let reconst = (predicted + dequantized).clamp(i16::MIN as i32, 32767) as i16;

		lms.update(reconst, dequantized);
		(quantized, dequantized, reconst)
	}
}

impl SliceScaler for LinearScaler {
	fn scale(samples: &[i16], lms: &mut QoaLmsState, chn: usize, channel_count: usize) -> u64 {
		let len = SLICE_LEN.clamp(0, samples.len());
		let rng = chn..len * channel_count + chn;
		let (_, best_slice, best_lms) = (0..16).map(|sf| {
			let mut lms = *lms;
			let mut slice = sf as u64;
			let error = rng.clone()
						   .step_by(channel_count)
						   .map(|si| samples[si])
						   .fold(0, |acc, sample| {
							   let (quantized, _, reconst) =
								   Self::scale_sample(sample as i32, sf, &mut lms);

							   slice = slice << 3 | quantized as u64;
							   let mut error = sample as i64 - reconst as i64;
							   error *= error;
							   acc + error as u64
						   });
			(error, slice, lms)
		}).min_by_key(|(e, _, _)| *e).unwrap();

		*lms = best_lms;
		best_slice << (SLICE_LEN - len) * 3
	}
}

#[cfg(test)]
mod test {
	use quickcheck::{Arbitrary, Gen};
	use quickcheck_macros::quickcheck;
	use qoa_ref_sys::scale_slice;
	use crate::encoder::slice_scaler::{LinearScaler, SliceScaler};
	use crate::QoaLmsState;

	#[quickcheck]
	fn scale_sample(lms: QoaLmsState, sample: i16) {
		for sf in 0..16 {
			let (lin_quant, lin_dequant, lin_reconst) =
				LinearScaler::scale_sample(sample as i32, sf, &mut lms.clone());
			let (quant, dequant, reconst) =
				qoa_ref_sys::scale_sample(sample as i32, sf as i32, &mut lms.clone().into());
			assert_eq!(lin_quant,   quant,     "quantized residual for scale factor {sf}");
			assert_eq!(lin_dequant, dequant, "dequantized residual for scale factor {sf}");
			assert_eq!(lin_reconst, reconst, "reconstructed sample for scale factor {sf}");
		}
	}

	#[derive(Copy, Clone, Debug)]
	pub(crate) struct Slice(pub [i16; 40]);

	impl Arbitrary for Slice {
		fn arbitrary(g: &mut Gen) -> Self {
			let mut slice = [0; 40];
			slice.fill_with(|| i16::arbitrary(g));
			Self(slice)
		}
	}

	#[quickcheck]
	fn scale(Slice(ref slice): Slice, lms: QoaLmsState) {
		let mut lin_lms = [lms; 2];
		let mut ref_lms = [lms.into(); 8];
		let lin_slice1 = LinearScaler::scale(slice, &mut lin_lms[0], 0, 2);
		let lin_slice2 = LinearScaler::scale(slice, &mut lin_lms[1], 1, 2);
		let ref_slice1 = scale_slice(slice, 1, &mut ref_lms, 0);
		let ref_slice2 = scale_slice(slice, 2, &mut ref_lms, 1);
		assert_eq!(lin_lms[0], ref_lms[0].into(), "LMS state on channel 0");
		assert_eq!(lin_slice1, ref_slice1, "Slice data on channel 0");
		assert_eq!(lin_lms[1], ref_lms[1].into(), "LMS state on channel 1");
		assert_eq!(lin_slice2, ref_slice2, "Slice data on channel 1");
	}
}

#[cfg(feature = "simd")]
mod simd {
	use std::simd::{i32x16, SimdInt, SimdOrd, SimdUint, u8x16};
	use crate::encoder::slice_scaler::SliceScaler;
	use crate::{DEQUANT_TABLE, QoaLmsState, QUANT_TABLE, SLICE_LEN};
	use crate::simd::{const_splat, div, i64x16, LmsStateVector, SimdLanes, u64x16};

	/// A SIMD vector scaler. Computes the slice for each scale factor as a vector
	/// element, then chooses the scaled slice with the smallest error. Should be
	/// significantly faster than the linear scaler on modern platforms, namely AVX
	/// on x86 and SVE on ARM.
	pub struct VectorScaler;

	impl VectorScaler {
		fn scale_sample(sample: i32x16, lms: &mut LmsStateVector) -> (i32x16, i32x16, i32x16) {
			const SCALED_MIN: i32x16 = const_splat(-8);
			const SCALED_MAX: i32x16 = const_splat( 8);
			const SAMPLE_MIN: i32x16 = const_splat(-32768);
			const SAMPLE_MAX: i32x16 = const_splat( 32767);

			let prediction = lms.predict();
			let residual = sample - prediction;
			let scaled = div(residual);
			let clamped = scaled.simd_clamp(SCALED_MIN, SCALED_MAX) + SCALED_MAX;
			let quantized: i32x16 = u8x16::gather_or_default(
				&QUANT_TABLE,
				clamped.cast()
			).cast();
			let dequantized = {
				let mut q = i32x16::splat(0);
				for sf in 0..16 {
					q[sf] = DEQUANT_TABLE[sf][quantized[sf] as usize]
				}
				q
			};
			let reconst = (prediction + dequantized).simd_clamp(SAMPLE_MIN, SAMPLE_MAX);
			(quantized, dequantized, reconst)
		}
	}

	impl SliceScaler for VectorScaler {
		fn scale(samples: &[i16], lms: &mut QoaLmsState, chn: usize, channel_count: usize) -> u64 {
			const SFS: u64x16 = u64x16::from_array(
				[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
			);
			let len = SLICE_LEN.clamp(0, samples.len());
			let rng = chn..len * channel_count + chn;

			// Create an LMS State vector for all 16 scale factors.
			let mut lms_vec = LmsStateVector::from(*lms);
			let mut cur_err = u64x16::splat(0);
			let mut slice = SFS;

			// Compute the scaled slice and error for all scale factors, then pick
			// the slice with the lowest error.
			for si in rng.step_by(channel_count) {
				// Load the sample into a x16 vector.
				let sample = i32x16::splat(samples[si] as i32);
				let (quantized, dequantized, reconst) =
					Self::scale_sample(sample, &mut lms_vec);

				// Compute the error, square, and add to the error sum.
				let error: i64x16 = (sample - reconst).cast();
				cur_err += (error * error).cast();

				lms_vec.update(reconst, dequantized);
				slice = slice << u64x16::splat(3) | quantized.cast();
			}

			// Return the slice with the minimum error and assign its LMS.
			let best_lane = cur_err.min_lane();
			*lms = lms_vec.collapse(best_lane);
			slice[best_lane]
		}
	}

	#[cfg(test)]
	mod test {
		use std::simd::i32x16;
		use quickcheck_macros::quickcheck;
		use qoa_ref_sys::qoa::qoa_lms_t;
		use qoa_ref_sys::scale_slice;
		use crate::encoder::slice_scaler::{SliceScaler, VectorScaler};
		use crate::encoder::slice_scaler::test::Slice;
		use crate::QoaLmsState;
		use crate::simd::LmsStateVector;

		#[quickcheck]
		fn scale_sample(lms: QoaLmsState, sample: i16) {
			let ref mut vector_lms = LmsStateVector::from(lms.clone());
			let ref mut native_lms = vec![Into::<qoa_lms_t>::into(lms); 16];

			let (vec_quant, vec_dequant, vec_reconst) =
				VectorScaler::scale_sample(i32x16::splat(sample as i32), vector_lms);

			for sf in 0..16 {
				let ref mut lms = native_lms[sf];
				let (quant, dequant, reconst) =
					qoa_ref_sys::scale_sample(sample as i32, sf as i32, lms);
				assert_eq!(vec_quant  [sf],   quant as i32,   "quantized residual for scale factor {sf}");
				assert_eq!(vec_dequant[sf], dequant,        "dequantized residual for scale factor {sf}");
				assert_eq!(vec_reconst[sf], reconst as i32, "reconstructed sample for scale factor {sf}");
			}
		}

		#[quickcheck]
		fn scale(Slice(ref slice): Slice, lms: QoaLmsState) {
			let mut vec_lms = [lms; 2];
			let mut ref_lms = [lms.into(); 8];
			let vec_slice1 = VectorScaler::scale(slice, &mut vec_lms[0], 0, 2);
			let vec_slice2 = VectorScaler::scale(slice, &mut vec_lms[1], 1, 2);
			let ref_slice1 = scale_slice(slice, 1, &mut ref_lms, 0);
			let ref_slice2 = scale_slice(slice, 2, &mut ref_lms, 1);
			assert_eq!(vec_lms[0], ref_lms[0].into(), "LMS state on channel 0");
			assert_eq!(vec_slice1, ref_slice1, "Slice data on channel 0");
			assert_eq!(vec_lms[1], ref_lms[1].into(), "LMS state on channel 1");
			assert_eq!(vec_slice2, ref_slice2, "Slice data on channel 1");
		}
	}
}
