# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/MikeLuu99/metasearch-rust/compare/v0.2.0...v0.2.1) - 2026-08-05

### Added

- Sogou Images search engine on `GET /images`, ported from SearXNG's `sogou_images.py` (parses the page's embedded `window.__INITIAL_STATE__` JSON, extracts hosting page URL, image URL, thumbnail, snippet and source site)

### Fixed

- fix release-plz action ref to release-plz/action@v0.5

### Other

- switch to release-plz for automated crate releases
