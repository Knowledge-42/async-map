[![Build Status](https://cloud.drone.io/api/badges/herblet/async-map/status.svg)](https://cloud.drone.io/herblet/async-map)

This Rust project provides datastructures for shared use in an async environment, with the aim of avoiding locking as far as possible.

The first such datastructure is VersionedMap, a Map with no-Locking and optimized for high-read, low-write use cases.
