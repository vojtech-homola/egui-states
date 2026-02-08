# Changelog

## [0.11.0] - 2026-02-08

### 🚀 Features

- Remove error message from client to server [#25](https://github.com/vojtech-homola/egui-states/pull/25)

## [0.10.0] - 2026-01-27

### 🚀 Features

- Add atomics values [#24](https://github.com/vojtech-homola/egui-states/pull/24)
- Simplify states structure

## [0.9.1] - 2026-01-17

### 🐛 Bug Fixes

- Add manylinux in linux build
- Missing version file in wheel

## [0.9.0] - 2026-01-16

### 🚀 Features

- Use dynamic states server [#20](https://github.com/vojtech-homola/egui-states/pull/20)
- Add egui-states-widgets [#22](https://github.com/vojtech-homola/egui-states/pull/22)
- Protocol improvements [#23](https://github.com/vojtech-homola/egui-states/pull/23)

### 🐛 Bug Fixes

- All sort of fixes [#21](https://github.com/vojtech-homola/egui-states/pull/21)
- Fix error in merge
- Server stop/start

## [0.8.4] - 2025-10-29

### 🚀 Features

- Blocking images messages [#18](https://github.com/vojtech-homola/egui-states/pull/18)
- Update pyo3 to 0.27 [#19](https://github.com/vojtech-homola/egui-states/pull/19)

## [0.8.2] - 2025-09-23

### 🚀 Features

- Multi vs single in signals

### 🐛 Bug Fixes

- Bugs in clients reconnection

## [0.8.1] - 2025-09-19

### 🚀 Features

- List size comment in build script

### 🐛 Bug Fixes

- Make new parsing works
- Bugs in images and big data transfer

## [0.8.0] - 2025-09-16

### 🚀 Features

- Improve states parsing

### 🐛 Bug Fixes

- Bug in images serialization

## [0.7.0] - 2025-09-09

### 🚀 Features

- Use websockets [#17](https://github.com/vojtech-homola/egui-states/pull/17)

## [0.6.1] - 2025-08-12

### 🐛 Bug Fixes

- Use shape instead of size in ValueImage

## [0.6.0] - 2025-07-29

### 🐛 Bug Fixes

- Order of x,y data in Graph::from_graph_data [#15](https://github.com/vojtech-homola/egui-states/pull/15)
- More free pyo3 requirement
- Fix server default ip

### 🚜 Refactor

- New repository structure [#16](https://github.com/vojtech-homola/egui-states/pull/16)

## [0.5.2] - 2025-03-10

### 🚀 Features

- Use pyo3 version 0.24

## [0.5.1] - 2025-02-24

### 🚀 Features

- Use edition 2024 [#14](https://github.com/vojtech-homola/egui-states/pull/14)

## [0.5.0] - 2025-01-30

### 🚀 Features

- Allow python 3.11
- Move image initialization out of  crate [#13](https://github.com/vojtech-homola/egui-states/pull/13)

### 🐛 Bug Fixes

- Return back to python 3.12

## [0.4.2] - 2025-01-23

### 🐛 Bug Fixes

- Bug in signals - wrong argumets count
- Bug in build scripts fot list and dict

## [0.4.1] - 2025-01-15

### 🚀 Features

- Add Hash implementation for pyenums

### 🐛 Bug Fixes

- Bug in python signals

## [0.4.0] - 2025-01-15

### 🚀 Features

- Add image get size method
- Use serde for serialization/deserialization [#12](https://github.com/vojtech-homola/egui-states/pull/12)
- Modify the client builder parameters

### 🐛 Bug Fixes

- Fix typo in python structures
- Wrong version in Cargo.toml

## [0.3.0] - 2024-11-29

### 🚀 Features

- Use pyo3 0.23 [#8](https://github.com/vojtech-homola/egui-states/pull/8)
- Use super simple graphs [#9](https://github.com/vojtech-homola/egui-states/pull/9)
- Speed up and fix images [#10](https://github.com/vojtech-homola/egui-states/pull/10)
- Enums act as normal values, statics and signals [#11](https://github.com/vojtech-homola/egui-states/pull/11)

### 🐛 Bug Fixes

- Remove special character from name

## [0.2.2] - 2024-11-15

### 🚀 Features

- Add f32 into basic values
- Control client connection from UI [#7](https://github.com/vojtech-homola/egui-states/pull/7)
- Signals callbacks can return any value

## [0.2.0] - 2024-11-05

### 🚀 Features

- Improve acces to colections
- Use FromPyObject trait [#5](https://github.com/vojtech-homola/egui-states/pull/5)
- *(graphs)* Use general graphs [#4](https://github.com/vojtech-homola/egui-states/pull/4)
- Use macros for creating states [#6](https://github.com/vojtech-homola/egui-states/pull/6)

### 🐛 Bug Fixes

- Processing command messages in client + name threads
- *(python)* Reuse states classes and finish types in python
- Bugs in build script

### 🚜 Refactor

- Refactor items transport

## [0.1.1] - 2024-10-07

### 🚀 Features

- Use no hasher for u32 id keys

### 🐛 Bug Fixes

- Bug in python server

### 🚜 Refactor

- Simplify client creation

## [0.1.0] - 2024-10-06

### 🚀 Features

- Use handshake and setting address/port
- Add version, license and readme

### 🐛 Bug Fixes

- First working version
- Fix README name
