[![Crates.io](https://img.shields.io/crates/v/async-map.svg?logo=rust)](https://crates.io/crates/async-map)
![async-map](https://github.com/herblet/async-map/actions/workflows/build_with_coverage.yml/badge.svg)
[![codecov](https://codecov.io/gh/Knowledge-42/async-map/branch/main/graph/badge.svg?token=I579HJZVHQ)](https://codecov.io/gh/Knowledge-42/async-map)

This Rust project provides datastructures for shared use in an async environment, with the aim of avoiding locking as far as possible.

The first such datastructure is VersionedMap, a Map with no-Locking and optimized for high-read, low-write use cases.
