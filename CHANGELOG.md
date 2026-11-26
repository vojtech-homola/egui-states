# Changelog

## [0.17.0] - 2026-11-26

### 🚀 Features

- Improve examples [#63](https://github.com/vojtech-homola/egui-states/pull/63)
- Improve project testing and fix any bugs which ware discovered [#64](https://github.com/vojtech-homola/egui-states/pull/64)
- Add C ABI interface for state server [#65](https://github.com/vojtech-homola/egui-states/pull/65)
- Add support for serving static web pages alongside state server [#66](https://github.com/vojtech-homola/egui-states/pull/66)
- Add macOS and Python 3.15 support [#68](https://github.com/vojtech-homola/egui-states/pull/68)
- Add CI/CD workflows for GitHub Actions and Dependabot configuration [#69](https://github.com/vojtech-homola/egui-states/pull/69)
- Add a configurable server heartbeat [#75](https://github.com/vojtech-homola/egui-states/pull/75)
- [**breaking**] Improve static web page serving [#76](https://github.com/vojtech-homola/egui-states/pull/76)
- [**breaking**] Add typed Data headers and restructure the server and client cores [#77](https://github.com/vojtech-homola/egui-states/pull/77)
- [**breaking**] Give Data a "not set" state and require headers to create data [#78](https://github.com/vojtech-homola/egui-states/pull/78)
- Add data transfer channels [#79](https://github.com/vojtech-homola/egui-states/pull/79)
- Enhance Python server example with interactive mode and web serving capabilities
- Add optional TLS for the server and native client [#80](https://github.com/vojtech-homola/egui-states/pull/80)
- [**breaking**] Lower state latency during image and data transfers [#81](https://github.com/vojtech-homola/egui-states/pull/81)
- Let the server control which client may connect and tell clients why they were closed [#82](https://github.com/vojtech-homola/egui-states/pull/82)
- Set initial state values with #[state(init = ...)] [#83](https://github.com/vojtech-homola/egui-states/pull/83)
- [**breaking**] Store server images in their own format and share the transfer scheduler with Data [#85](https://github.com/vojtech-homola/egui-states/pull/85)
- [**breaking**] Expand synchronized state types and client behavior [#87](https://github.com/vojtech-homola/egui-states/pull/87)

### 🐛 Bug Fixes

- Resolve hangs, races and panics found in code review [#67](https://github.com/vojtech-homola/egui-states/pull/67)
- Resolve CI failures and clippy warnings [#73](https://github.com/vojtech-homola/egui-states/pull/73)
- Bug fixes from code review [#74](https://github.com/vojtech-homola/egui-states/pull/74)

### 🚜 Refactor

- Reorder parameters in set, add, replace, and remove methods for consistency

### 🧪 Testing

- Wait for the low-priority image removal in images-wire [#84](https://github.com/vojtech-homola/egui-states/pull/84)

## [0.16.0] - 2026-08-24

### 🚀 Features

- Add basic documentation (AI generated) [#56](https://github.com/vojtech-homola/egui-states/pull/56)
- Edit max message size limits [#58](https://github.com/vojtech-homola/egui-states/pull/58)
- Improve documentation [#59](https://github.com/vojtech-homola/egui-states/pull/59)
- Add option to add derive traits for custom types [#61](https://github.com/vojtech-homola/egui-states/pull/61)
- Add ImageMulti state [#60](https://github.com/vojtech-homola/egui-states/pull/60)
- Allow restart server with different parameters [#62](https://github.com/vojtech-homola/egui-states/pull/62)

### 🐛 Bug Fixes

- Improve sending ack in case of the failure on client side [#57](https://github.com/vojtech-homola/egui-states/pull/57)

## [0.15.0] - 2026-08-05

### 🚀 Features

- Add .zed folder into gitignore [#41](https://github.com/vojtech-homola/egui-states/pull/41)
- Improve images transport [#42](https://github.com/vojtech-homola/egui-states/pull/42)
- Add DataMultiTake state [#43](https://github.com/vojtech-homola/egui-states/pull/43)
- Improve logging on server and add client messages [#46](https://github.com/vojtech-homola/egui-states/pull/46)
- Bump PyO3 version to 0.29 [#49](https://github.com/vojtech-homola/egui-states/pull/49)
- Improve authentication [#48](https://github.com/vojtech-homola/egui-states/pull/48)
- Simplify takes values and refactor Event [#50](https://github.com/vojtech-homola/egui-states/pull/50)
- Add rust server - experimental [#51](https://github.com/vojtech-homola/egui-states/pull/51)
- Add maximal message size [#52](https://github.com/vojtech-homola/egui-states/pull/52)
- Add previous values for Value callbacks [#53](https://github.com/vojtech-homola/egui-states/pull/53)
- Add set all method to the Image [#55](https://github.com/vojtech-homola/egui-states/pull/55)

### 🐛 Bug Fixes

- Minor fixes in client initialization [#54](https://github.com/vojtech-homola/egui-states/pull/54)

### 🧪 Testing

- Improve example structure [#44](https://github.com/vojtech-homola/egui-states/pull/44)

## [0.14.0] - 2026-05-13

### 🚀 Features

- Improve type checking [#35](https://github.com/vojtech-homola/egui-states/pull/35)
- Add ValueTake [#36](https://github.com/vojtech-homola/egui-states/pull/36)
- General Data states [#32](https://github.com/vojtech-homola/egui-states/pull/32)
- Add DataMulti [#38](https://github.com/vojtech-homola/egui-states/pull/38)
- Add DataTake [#39](https://github.com/vojtech-homola/egui-states/pull/39)

### 🐛 Bug Fixes

- Fix bugs in collections [#34](https://github.com/vojtech-homola/egui-states/pull/34)
- DataTake incorrectly synchronizes [#40](https://github.com/vojtech-homola/egui-states/pull/40)

## [0.13.3] - 2026-03-27

### 🐛 Bug Fixes

- Bugs in grpahs on server side [#33](https://github.com/vojtech-homola/egui-states/pull/33)

## [0.13.2] - 2026-03-11

### 🐛 Bug Fixes

- Bug in processing signals for callbacks

## [0.13.1] - 2026-03-10

### 🚀 Features

- Improve debugging with values names [#31](https://github.com/vojtech-homola/egui-states/pull/31)

## [0.13.0] - 2026-03-09

### 🚀 Features

- Improve atomics values [#28](https://github.com/vojtech-homola/egui-states/pull/28)

### 🚜 Refactor

- Change project structure [#29](https://github.com/vojtech-homola/egui-states/pull/29)

## [0.12.0] - 2026-03-04

### 🚀 Features

- Improve histogram range selection
- Support atomics for enums [#27](https://github.com/vojtech-homola/egui-states/pull/27)

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
