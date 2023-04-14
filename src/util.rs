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

use std::ops::Mul;

pub trait Then: Sized {
	fn then_ok<T, E: Default>(self, val: T) -> Result<T, E>;

	fn then_err<T: Default, E>(self, err: E) -> Result<T, E>;

	fn and_then<T, F: FnMut() -> Option<T>>(self, then: F) -> Option<T>;
}

impl Then for bool {
	fn then_ok<T, E: Default>(self, val: T) -> Result<T, E> {
		if self { Ok(val) } else { Err(E::default()) }
	}

	fn then_err<T: Default, E>(self, err: E) -> Result<T, E> {
		if self { Err(err) } else { Ok(T::default()) }
	}

	fn and_then<T, F: FnMut() -> Option<T>>(self, mut then: F) -> Option<T> {
		if self { then() } else { None }
	}
}

pub trait Zip<T>: Iterator<Item = (T, T)> + Sized {
	fn mul(self) -> impl Iterator<Item = T::Output> where T: Mul {
		self.map(|(a, b)| a * b)
	}
}

impl<T, I: Iterator<Item = (T, T)> + Sized> Zip<T> for I { }
