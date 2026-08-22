# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.4](https://github.com/MikeLuu99/metasearch-rust/compare/v0.3.3...v0.3.4) - 2026-08-22

### Other

- shrink release binary (strip, lto, codegen-units=1)

## [0.3.3](https://github.com/MikeLuu99/metasearch-rust/compare/v0.3.2...v0.3.3) - 2026-08-22

### Fixed

- startpage bot-wall detection and result pagination

### Other

- apply cargo fmt

## [0.3.2](https://github.com/MikeLuu99/metasearch-rust/compare/v0.3.1...v0.3.2) - 2026-08-22

### Fixed

- engine reliability and result correctness
- harden server exposure, flow control, and lifecycle
- correct documentation drift
- only strip index filenames preceded by a path separator
- avoid UTF-8 panic when truncating snippets in examples
- return ParseFailed when Sogou omits initial-state JSON

### Other

- apply cargo fmt
- add search-tui dependency

## [0.3.1](https://github.com/MikeLuu99/metasearch-rust/compare/v0.3.0...v0.3.1) - 2026-08-07

### Other

- embed load-test queries in locustfile

## [0.3.0](https://github.com/MikeLuu99/metasearch-rust/compare/v0.2.1...v0.3.0) - 2026-08-07

### Other

- add locust load test harness
- add response cache, single-flight, and per-engine flow control
- add run instructions to examples
- auto-publish on release PR merge (release-plz default)
- publish only on version tag push

## [0.2.1](https://github.com/MikeLuu99/metasearch-rust/compare/v0.2.0...v0.2.1) - 2026-08-05

### Added

- Sogou Images search engine on `GET /images`, ported from SearXNG's `sogou_images.py` (parses the page's embedded `window.__INITIAL_STATE__` JSON, extracts hosting page URL, image URL, thumbnail, snippet and source site)

### Fixed

- fix release-plz action ref to release-plz/action@v0.5

### Other

- switch to release-plz for automated crate releases
