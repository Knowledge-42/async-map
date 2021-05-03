This Rust project provides datastructures for shared use in an async environment, with the aim of avoiding locking completely.

The first such datastructure is VersionedMap, a Map with no-Locking and optimized for high-read, low-write use cases.
