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
	use std::simd::{i32x16, i32x4, i64x8, LaneCount, Simd, SimdElement, SimdInt, SupportedLaneCount, u64x8};
	use crate::encoder::slice_scaler::SliceScaler;
	use crate::{DEQUANT_TABLE, QoaLmsState, QUANT_TABLE, SLICE_LEN, vec_div};

	/// A SIMD vector scaler. Computes the slice for each scale factor as a vector
	/// element, then chooses the scaled slice with the smallest error. Should be
	/// significantly faster than the linear scaler on modern platforms, namely AVX
	/// on x86 and SVE on ARM.
	pub struct VectorScaler;

	impl VectorScaler {
		fn split<const N: usize, T: SimdElement>(v: Simd<T, N>) -> (Simd<T, { N / 2 }>, Simd<T, { N / 2 }>)
																where LaneCount<N>: SupportedLaneCount,
																	  LaneCount<{ N / 2 }>: SupportedLaneCount {
			let v = v.to_array();
			let (lo, hi) = v.split_at(N / 2);
			(Simd::from_slice(lo), Simd::from_slice(hi))
		}

		fn join_into_array<const N: usize, T: SimdElement + Default>(
			lo: Simd<T, N>,
			hi: Simd<T, N>
		) -> [T; N * 2] where LaneCount<N>: SupportedLaneCount {
			let mut array = [T::default(); N * 2];
			array[..8].copy_from_slice(lo.as_array());
			array[8..].copy_from_slice(hi.as_array());
			array
		}

		fn scale_sample(sample: i32x16, lms: &mut LmsStateVector) -> (i32x16, i32x16, i32x16) {
			let prediction = lms.predict();
			let residual = sample - prediction;
			let scaled = vec_div(residual);
			let clamped = scaled.clamp(
				i32x16::splat(-8),
				i32x16::splat( 8)
			);
			let quantized = {
				let mut q = clamped + i32x16::splat(8);
				for i in 0..16 {
					q[i] = QUANT_TABLE[q[i] as usize] as i32;
				}
				q
			};
			let dequantized = {
				let mut dq = quantized;
				for i in 0..16 {
					dq[i] = DEQUANT_TABLE[i][dq[i] as usize];
				}
				dq
			};
			let reconst = (prediction + dequantized).clamp(
				i32x16::splat(-32768),
				i32x16::splat( 32767)
			);
			(quantized, dequantized, reconst)
		}
	}

	impl SliceScaler for VectorScaler {
		fn scale(samples: &[i16], lms: &mut QoaLmsState, chn: usize, channel_count: usize) -> u64 {
			let len = SLICE_LEN.clamp(0, samples.len());
			let rng = chn..len * channel_count + chn;

			// Create an LMS State vector for all 16 scale factors.
			let mut lms_vec = LmsStateVector::from(*lms);

			let mut cur_err1 = u64x8::splat(0);
			let mut cur_err2 = u64x8::splat(0);

			let mut slice1 = u64x8::from_array([0, 1,  2,  3,  4,  5,  6,  7]);
			let mut slice2 = u64x8::from_array([8, 9, 10, 11, 12, 13, 14, 15]);

			// Compute the scaled slice and error for all scale factors, then pick
			// the slice with the lowest error.
			for si in rng.step_by(channel_count) {
				// Load the sample into a x16 vector.
				let sample = i32x16::splat(samples[si] as i32);
				let (quantized, dequantized, reconst) =
					Self::scale_sample(sample, &mut lms_vec);

				// Compute the error, split, square, and add to the error sum.
				let error = (sample - reconst).cast();
				let (error1, error2): (i64x8, _) = Self::split(error);
				cur_err1 += (error1 * error1).cast();
				cur_err2 += (error2 * error2).cast();

				lms_vec.update(reconst, dequantized);
				let (qr1, qr2) = Self::split(quantized);
				slice1 = slice1 << u64x8::splat(3) | qr1.cast();
				slice2 = slice2 << u64x8::splat(3) | qr2.cast();
			}

			// Return the slice with the minimum error and assign its LMS.
			let errors = Self::join_into_array(cur_err1, cur_err2);
			let slices = Self::join_into_array(slice1, slice2);
			let best_i = errors.iter().enumerate().min_by_key(|(_, e)| *e).unwrap().0;
			*lms = lms_vec.collapse(best_i);
			slices[best_i]
		}
	}

	#[derive(Copy, Clone)]
	struct LmsStateVector {
		history: [i32x4; 16],
		weights: [i32x4; 16]
	}

	impl LmsStateVector {
		fn predict(&self) -> i32x16 {
			let Self { history, weights } = self;
			let mut prediction = [0; 16];
			for i in 0..16 {
				prediction[i] = (history[i] * weights[i]).reduce_sum() >> 13;
			}

			i32x16::from_array(prediction)
		}

		fn update(&mut self, samples: i32x16, residual: i32x16) {
			let Self { history, weights } = self;

			for lms in 0..16 {
				let history = &mut history[lms];
				let weights = &mut weights[lms];
				let sample = samples[lms];
				let delta = residual[lms] >> 4;
				for i in 0..4 {
					weights[i] += if history[i] < 0 {
						-delta
					} else {
						delta
					};
				}

				*history = history.rotate_lanes_left::<1>();
				history[3] = sample as i32;
			}
		}

		fn collapse(self, sf: usize) -> QoaLmsState {
			QoaLmsState {
				history: self.history[sf].to_array(),
				weights: self.weights[sf].to_array(),
			}
		}
	}

	impl From<QoaLmsState> for LmsStateVector {
		fn from(QoaLmsState { history, weights }: QoaLmsState) -> Self {
			Self {
				history: [i32x4::from_array(history); 16],
				weights: [i32x4::from_array(weights); 16]
			}
		}
	}

	#[cfg(test)]
	mod test {
		use std::simd::i32x16;
		use quickcheck_macros::quickcheck;
		use qoa_ref_sys::qoa::qoa_lms_t;
		use qoa_ref_sys::scale_slice;
		use crate::encoder::slice_scaler::{SliceScaler, VectorScaler};
		use crate::encoder::slice_scaler::simd::LmsStateVector;
		use crate::encoder::slice_scaler::test::Slice;
		use crate::QoaLmsState;

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
