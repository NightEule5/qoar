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
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
	println!("cargo:rerun-if-changed=qoa-ref-codec/qoaconv.c");
	println!("cargo:rerun-if-changed=src/qoa.rs");

	if !Path::new("qoa-ref-codec").exists() {
		panic!(
			"could not find reference codec submodule; run `git submodule init` \
			then `git submodule update`"
		)
	}

	bindgen::Builder::default()
		.header("qoa-ref-codec/qoaconv.c")
		.layout_tests(false)
		.allowlist_type("qoa_(desc|lms_t)")
		.allowlist_function("(qoa|qoaconv)_(wav_(read|write)|(encode|decode)(_frame|_header)?)")
		.merge_extern_blocks(true)
		.raw_line("#![allow(dead_code)]")
		.generate()?
		.write_to_file("src/qoa.rs")?;

	cc::Build::default()
		.no_default_flags(false)
		.compiler("gcc")
		.file("qoa-ref-codec/qoaconv.c")
		.flag("-std=gnu99")
		.flag("-O3")
		.flag("-lm")
		.flag("-w")
		.compile("qoa");

	println!("cargo:rustc-link-lib=qoa");
	Ok(())
}
