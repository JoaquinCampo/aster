# Third-party notices

Aster is designed to invoke the Pi runtime from [`badlogic/pi-mono`](https://github.com/badlogic/pi-mono).
Pi is licensed under the MIT License:

> Copyright (c) 2025 Mario Zechner

When Pi source or substantial portions are distributed with Aster, its copyright notice and MIT license text must accompany that distribution. Aster currently does not vendor Pi source or binaries; it discovers or invokes a separately installed runtime.

Rust and JavaScript dependencies retain their respective licenses. Release packaging must run dependency-license and notice generation before bundled third-party artifacts are distributed.
