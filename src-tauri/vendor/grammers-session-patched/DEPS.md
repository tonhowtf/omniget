# Dependencies

## grammers-tl-types

Used for dealing with correct update processing.

The serialization and deserialization traits are also used for storing and loading the session
into and from bytes.

## grammers-tl-gen

Used to generate Rust code for a custom Type Language definition which defines the serialized
session format.

## grammers-tl-parser

Used to parse the custom Type Language definition used for the session itself.

## log

Used to log messages during update processing.

## toml

Used to test that this file lists all dependencies from `Cargo.toml`.

## serde

Support serde ecosystem.

## serde_derive

Macros that auto generate serde code.

## serde_bytes

Use better bytes encode/decode pattern in serde.

## sqlite

SQLite-based session storage.
