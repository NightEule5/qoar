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

//! SIMD primitives for vector-based encoding.

#![cfg(feature = "simd")]
#![allow(non_camel_case_types)]

use std::simd::{i32x16, i32x4, i64x4, LaneCount, Simd, SimdElement, SimdInt, SimdOrd, SimdUint, SupportedLaneCount};
use crate::{QoaLmsState, RECIP_TABLE};

pub type i64x16 = Simd<i64, 16>;
pub type u64x16 = Simd<u64, 16>;

pub const fn const_splat<const N: usize, T: SimdElement>(v: T) -> Simd<T, N>
	where LaneCount<N>: SupportedLaneCount {
	// Should sidestep rust#97804 since this is done at compile-time.
	// https://github.com/rust-lang/rust/issues/97804
	Simd::from_array([v; N])
}

// div

const RECIP_VEC: i64x16 = i64x16::from_array(RECIP_TABLE);

const ZERO: i32x16 = const_splat(0);
const ONE : i32x16 = const_splat(1);

// Maps each lane to 1 if less than zero, otherwise 0.
fn lt_zero(v: i32x16) -> i32x16 {
	v.is_negative().select(ONE, ZERO)
}

// Maps each lane to 1 if greater than zero, otherwise 0.
fn gt_zero(v: i32x16) -> i32x16 {
	v.is_positive().select(ONE, ZERO)
}

pub fn div(v: i32x16) -> i32x16 {
	const ADD: i64x16 = const_splat(1 << 15);
	const SHR: i64x16 = const_splat(16);
	let mut n = ((v.cast() * RECIP_VEC + ADD) >> SHR).cast();

	n += (gt_zero(v) - lt_zero(v)) -
		 (gt_zero(n) - lt_zero(n));
	n
}

// min

pub trait SimdLanes<T> {
	fn min_lane(self) -> usize;
}

impl SimdLanes<u64> for u64x16 {
	fn min_lane(self) -> usize {
		let min = self.reduce_min();
		self.to_array()
			.into_iter()
			.position(|v| v == min)
			.unwrap()
	}
}

// lms

#[derive(Copy, Clone)]
pub struct LmsState {
	history: i32x4,
	weights: i32x4
}

impl LmsState {
	pub fn predict(&self) -> i32 {
		((self.history() * self.weights()).reduce_sum() >> 13) as i32
	}

	pub fn update(&mut self, sample: i32, residual: i32) {
		let Self { history, weights } = self;
		*weights += history.is_negative()
						   .select(
							   i32x4::splat(-(residual >> 4)),
							   i32x4::splat(  residual >> 4));
		*history = history.rotate_lanes_left::<1>();
		history[3] = sample;
	}

	fn history(&self) -> i64x4 { self.history.cast() }
	fn weights(&self) -> i64x4 { self.weights.cast() }
}

impl From<LmsState> for QoaLmsState {
	fn from(LmsState { history, weights }: LmsState) -> Self {
		Self {
			history: history.to_array(),
			weights: weights.to_array()
		}
	}
}

impl From<QoaLmsState> for LmsState {
	fn from(QoaLmsState { history, weights }: QoaLmsState) -> Self {
		Self {
			history: i32x4::from_array(history),
			weights: i32x4::from_array(weights)
		}
	}
}

#[derive(Copy, Clone)]
pub struct LmsStateVector([LmsState; 16]);

impl LmsStateVector {
	fn new(state: LmsState) -> Self { Self([state; 16]) }

	pub fn predict(&self) -> i32x16 {
		let Self(lms_array) = self;
		i32x16::from_array(
			lms_array.map(|lms| lms.predict())
		)
	}

	pub fn update(&mut self, sample: i32x16, residual: i32x16) {
		let Self(lms_array) = self;
		for sf in 0..16 {
			lms_array[sf].update(sample[sf], residual[sf]);
		}
	}

	pub fn collapse(self, sf: usize) -> QoaLmsState {
		let Self(lms_array) = self;
		lms_array[sf].into()
	}
}

impl From<QoaLmsState> for LmsStateVector {
	fn from(value: QoaLmsState) -> Self {
		Self::new(LmsState::from(value))
	}
}

#[cfg(test)]
mod test {
	use std::simd::i32x16;
	use quickcheck::{Arbitrary, Gen, TestResult};
	use quickcheck_macros::quickcheck;
	use crate::{DEQUANT_TABLE, qc_assert_eq, QoaLmsState};
	use crate::simd::LmsStateVector;

	#[derive(Copy, Clone, Debug)]
	struct ArbArray<T: Arbitrary, const N: usize>([T; N]);

	impl<T: Arbitrary + Copy + Default, const N: usize> Arbitrary for ArbArray<T, N> {
		fn arbitrary(g: &mut Gen) -> Self {
			let mut arr = [T::default(); N];
			arr.fill_with(|| T::arbitrary(g));
			Self(arr)
		}
	}

	#[quickcheck]
	fn vec_div(ArbArray(values): ArbArray<i32, 16>) {
		let vec = super::div(i32x16::from_array(values.clone()));
		let lin = values.into_iter()
						.enumerate()
						.map(|(i, v)| crate::div(v, i))
						.collect::<Vec<_>>();
		assert_eq!(vec, i32x16::from_slice(&lin))
	}

	#[quickcheck]
	fn lms_predict(lms: QoaLmsState) {
		let vec_lms = LmsStateVector::from(lms.clone());
		let lin_lms = vec![lms; 16];
		let vec = vec_lms.predict().to_array().to_vec();
		let lin = lin_lms.into_iter().map(|lms| lms.predict()).collect::<Vec<_>>();
		assert_eq!(vec, lin);
	}

	#[quickcheck]
	fn lms_update(lms: QoaLmsState, sample: i16, qr: usize) -> TestResult {
		if qr >= 8 { return TestResult::discard() }

		let mut vec_lms = LmsStateVector::from(lms);
		let mut lin_lms = vec![lms; 16];

		let mut residual = [0; 16];
		for sf in 0..16 {
			let r = DEQUANT_TABLE[sf][qr];
			residual[sf] = r;

			lin_lms[sf].update(sample, r);
		}

		vec_lms.update(
			i32x16::splat(sample as i32),
			i32x16::from_slice(&residual)
		);

		qc_assert_eq!(vec_lms.0.map(QoaLmsState::from).to_vec(), lin_lms)
	}
}
