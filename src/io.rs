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

use std::collections::VecDeque;
use std::error::Error;
use std::io;
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use amplify_derive::Display;
use crate::io::ReadError::Eof;

#[derive(Debug, Display)]
pub enum ReadError {
	#[display("unknown IO error")]
	Io(io::Error),
	#[display("end of stream reached prematurely")]
	Eof,
	#[display("{0}")]
	Other(Box<dyn Error>)
}

impl Error for ReadError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			Self::Io(ref err) => Some(err),
			Eof               => None,
			Self::Other(err)  => Some(err.as_ref())
		}
	}
}

#[derive(Debug, Display)]
pub enum WriteError {
	#[display("unknown IO error")]
	Io(io::Error),
	#[display("{0}")]
	Other(Box<dyn Error>)
}

impl Error for WriteError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			Self::Io(ref err) => Some(err),
			Self::Other(err)  => Some(err.as_ref())
		}
	}
}

impl From<io::Error> for ReadError {
	fn from(value: io::Error) -> Self {
		if let io::ErrorKind::UnexpectedEof = value.kind() {
			Eof
		} else {
			ReadError::Io(value)
		}
	}
}

impl From<io::Error> for WriteError {
	fn from(value: io::Error) -> Self { Self::Io(value) }
}

pub type ReadResult = Result<u64, ReadError>;
pub type WriteResult = Result<(), WriteError>;

/// An input stream of big endian, 64-bit integers.
pub trait SourceStream {
	fn read_long(&mut self) -> ReadResult;
}

pub trait IntoSourceStream {
	type Source: SourceStream;

	fn into_source(self) -> Self::Source;
}

impl<S: SourceStream> IntoSourceStream for S {
	type Source = Self;

	fn into_source(self) -> Self { self }
}

/// An output stream of big endian, 64-bit integers.
pub trait SinkStream {
	fn write_long(&mut self, value: u64) -> WriteResult;

	fn flush(&mut self) -> WriteResult { Ok(()) }
}

pub trait IntoSinkStream {
	type Sink: SinkStream;

	fn into_sink(self) -> Self::Sink;
}

impl<S: SinkStream> IntoSinkStream for S {
	type Sink = Self;

	fn into_sink(self) -> Self { self }
}

impl<R: Read> SourceStream for R {
	fn read_long(&mut self) -> ReadResult {
		let mut buf = [0; 8];
		self.read_exact(&mut buf)?;
		Ok(u64::from_be_bytes(buf))
	}
}

impl<W: Write> SinkStream for W {
	fn write_long(&mut self, value: u64) -> WriteResult {
		Ok(self.write_all(&value.to_be_bytes())?)
	}
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Buffer(VecDeque<u64>);

impl Buffer {
	pub fn unwrap(self) -> VecDeque<u64> { self.0 }

	pub fn encode(self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(self.len() * 8);

		for val in self.unwrap() {
			buf.extend_from_slice(&val.to_be_bytes());
		}

		buf
	}

	pub fn decode(buf: &mut Vec<u8>) -> Self {
		let len = buf.len() - buf.len() % 8;
		buf.drain(..len)
		   .array_chunks::<8>()
		   .map(u64::from_be_bytes)
		   .collect()
	}
}

impl SourceStream for Buffer {
	fn read_long(&mut self) -> ReadResult {
		self.pop_front().ok_or(Eof)
	}
}

impl SinkStream for Buffer {
	fn write_long(&mut self, value: u64) -> WriteResult {
		self.push_back(value);
		Ok(())
	}
}

impl From<VecDeque<u64>> for Buffer {
	fn from(value: VecDeque<u64>) -> Self { Buffer(value) }
}

impl From<Vec<u64>> for Buffer {
	fn from(value: Vec<u64>) -> Self { Buffer(value.into()) }
}

impl FromIterator<u64> for Buffer {
	fn from_iter<T: IntoIterator<Item = u64>>(iter: T) -> Self {
		Self(iter.into_iter().collect())
	}
}

impl Deref for Buffer {
	type Target = VecDeque<u64>;

	fn deref(&self) -> &Self::Target { &self.0 }
}

impl DerefMut for Buffer {
	fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}
