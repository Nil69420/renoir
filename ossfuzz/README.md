# OSS-Fuzz submission files

These files are the project files OSS-Fuzz expects under `projects/renoir/` in https://github.com/google/oss-fuzz. Submitting them is a step for the repository owner:

1. Replace `primary_contact` in `project.yaml` with a Google-account email that should receive bug reports.
2. Copy `project.yaml`, `Dockerfile` and `build.sh` into `projects/renoir/` of a fork of google/oss-fuzz and open a pull request (the OSS-Fuzz documentation, "New project guide", lists the review criteria).
3. Check locally first with the OSS-Fuzz helper: `python infra/helper.py build_fuzzers renoir` and `python infra/helper.py check_build renoir`.

The four targets are the cargo-fuzz targets in `fuzz/fuzz_targets` (lean ring, broadcast ring, frame ring, control info and sensor validators). Nothing here has been run against OSS-Fuzz's infrastructure yet.
