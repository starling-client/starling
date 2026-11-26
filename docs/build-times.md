# Build times

`tools/build_times.py` measures how long Sunrise takes to build, the same way on any checkout, so that two
machines or two commits can be compared. It only measures; it does not change the build.

## What it measures

Every build is the one a developer runs, `cargo build --locked -p sunrise-game --profile <profile> --timings`
(the game and `sunrise-ctl`), in a target folder of its own.

- **Cold:** a build from an empty target folder, for the `dev` and `fast` profiles. Its timings page gives the
  time of each crate and the critical path: the chain of units, each one waiting for the one before it, that
  ended last. Shortening any other unit does not shorten the build.
- **Nothing changed:** the same build again. This is the least any build costs.
- **Incremental:** a one-line change in one file of `sunrise-world`, `sunrise-client-render`,
  `sunrise-client` (`game.rs`, the game binary) and `sunrise-items` (a small crate low in the tree), then
  the build. The report lists the crates cargo recompiled and the slowest of them.

## Run it

```sh
python3 tools/build_times.py run                       # everything: 20 to 40 minutes
python3 tools/build_times.py run --profiles dev        # one profile
python3 tools/build_times.py run --only incremental --crates sunrise-items --profiles dev   # a quick check
python3 tools/build_times.py run --only cold --profiles dev
```

It writes `.local/build-times/reports/<date>-<commit>-<os>.json` (with `-2`, `-3`... when an earlier run took
that name) and a `.md` table beside it, and prints the table. Logs of every cargo call are in
`.local/build-times/logs/`.

Before you run it:

- Close other builds, tests and heavy programs. A cargo or rustc process running beside it, or a CPU that is
  more than 15% busy before a build, is warned about and kept with that number in the report. A measurement
  under load is a worse one; do it again when the machine is quiet.
- The working tree must be clean: the numbers are about a commit. `--allow-dirty` overrides this and the report
  says so.
- `CARGO_BUILD_JOBS` is respected (`--jobs N` overrides it); the report records the jobs and where they
  came from. `CARGO_TARGET_DIR` is not used: the tool builds in `.local/build-times/target-<profile>` and
  leaves yours alone. It only ever empties a folder it made itself, and never `target/`.
- `--repeats 3` runs each incremental measurement three times and reports the median.
- `--file sunrise-items=crates/sunrise-items/src/enchant.rs` edits another file. Any crate of the workspace
  can go in `--crates`; one that is not in the game's dependencies is reported as not recompiled.

## The edit

The incremental builds add one comment line (`// build-times probe ...`) to a file. A comment cannot change
behaviour, yet the file really changes. `--edit` says where it goes:

- `middle` (the default): before the top-level item nearest the middle of the file that follows a blank line
  (not a doc comment or an attribute); if the file has no such item, the comment goes on line 1, as
  with `top`, and the report warns. Every line below the comment moves, which is what an edit there does to
  rustc's incremental cache: the code below must be compiled again. The report gives the line (`file:line`).
- `top`: the first line. All the file's code moves.
- `append`: the last line. Almost nothing moves, so rustc has little to do. It shows how cheap a build can be.
- `touch`: no change to the bytes, only a new modification time.

Cargo decides on the file's modification time, not its content: after any of the four it runs rustc for the
crate, and for every crate that depends on it, because their input was written again. What rustc then
recompiles depends on what moved.

For a fair start, each measurement is preceded by a build of the unedited file in the same target folder. After the
measured build the file is put back byte for byte and built once more (not timed), then its old modification
time is restored. A build that fails is followed by that build of the unedited file too, so the next measurement
starts from it. The file and its modification time are put back on Ctrl+C, on an error, on SIGTERM and on
Ctrl+Break (Windows). If the process is killed outright, a note in `.local/build-times/` lets the next run
restore the file first. Either way, a file that something else wrote in the meantime (your editor) is left as it
is, and the run stops and says so: putting the old bytes back would undo that work.

## Compare

```sh
python3 tools/build_times.py compare before.json after.json
```

A table of seconds before, after and the change, for the cold builds, the nothing-changed build, every
incremental build, every crate of the cold builds' slowest list, and the two critical paths. When the
CPU, OS, RAM, jobs, rustc, edit mode, edited files or build variables (`RUSTFLAGS`, `RUSTC_WRAPPER`...) differ
between the reports, it says so first: those seconds are not comparable.
