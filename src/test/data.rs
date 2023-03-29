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

use crate::QoaSlice;

static SLICE: QoaSlice = QoaSlice {
	quant: 9,
	data: [1, 2, 3, 4, 5, 6, 7, 6, 5, 4, 3, 2, 1, 2, 3, 4, 5, 6, 7, 6]
};

static PACKED: [u8; 8] = [
	0b10010010,
	0b10011100,
	0b10111011,
	0b11101011,
	0b00011010,
	0b00101001,
	0b11001011,
	0b10111110
];

#[test]
fn slice_pack() {
	assert_eq!(SLICE.pack(), PACKED)
}

#[test]
fn slice_unpack() {
	let exp_slice = QoaSlice {
		quant: 9,
		data: [1, 2, 3, 4, 5, 6, 7, 6, 5, 4, 3, 2, 1, 2, 3, 4, 5, 6, 7, 6]
	};
	let mut act_slice = QoaSlice::default();
	act_slice.unpack(PACKED);

	assert_eq!(act_slice, exp_slice)
}
