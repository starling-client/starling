#!/usr/bin/env python3
"""Mechanically split the item-76 CPAL capture code out of audio/mic.rs.

Run once on the review baseline (958539692a2), then format the touched Rust files.
The follow-up commit changes the backend behavior; this script only moves the code.
"""

from pathlib import Path
import re


root = Path(__file__).resolve().parents[1]
mic = root / "crates/sunrise-client-render/src/audio/mic.rs"
modules = mic.with_suffix("")
source = mic.read_text()
audio = '#[cfg(feature = "audio")]\n'
tests = '#[cfg(any(feature = "audio", test))]\n'

capture_start = source.index(audio + "struct CpalBackend;")
parse_start = source.index('#[cfg(any(all(feature = "audio", target_os = "macos"), test))]', capture_start)
parse_end = source.index("\n#[cfg(all(", parse_start)
capture_end = source.index("#[cfg(test)]\nmod tests", parse_end)
capture = source[capture_start:parse_start] + source[parse_end:capture_end].lstrip()
source = source[:capture_start] + source[parse_start:parse_end] + "\n" + source[capture_end:]
capture = capture.replace(audio, "").replace("struct CpalBackend;", "pub(super) struct CpalBackend;").replace("struct CpalDevice {", "pub(super) struct CpalDevice {")

config_start = source.index("/// Classifies an OS audio backend error")
config_end = source.index(tests + "enum ProbeConfigs", config_start)
rank_start = source.index(tests + "fn choose_best_config", config_end)
rank_end = source.index(tests + "fn select_config_for_device", rank_start)
config = (source[config_start:config_end] + source[rank_start:rank_end]).replace(tests, "")
config = re.sub(r"^(fn |enum |struct )", r"pub(super) \1", config, flags=re.M)
config = re.sub(r"^    (fn rank\(|(?:channels|min_rate|max_rate|format|rate): )", r"    pub(super) \1", config, flags=re.M)
source = source[:rank_start] + source[rank_end:]
source = source[:config_start] + tests + "mod cpal_config;\n" + tests + "use cpal_config::*;\n\n" + source[config_end:]
source = source.replace(audio + "use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};\n", audio + "mod cpal_capture;\n" + audio + "use cpal_capture::CpalBackend;\n", 1)

modules.mkdir(exist_ok=True)
with (modules / "cpal_capture.rs").open("x") as out:
    out.write("//! CPAL microphone capture.\n\nuse super::*;\nuse cpal::traits::{DeviceTrait, HostTrait, StreamTrait};\n\n" + capture)
with (modules / "cpal_config.rs").open("x") as out:
    out.write("//! CPAL input configurations and error classification.\n\n" + config)
mic.write_text(source)
