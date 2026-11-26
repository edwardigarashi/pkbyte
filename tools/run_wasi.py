#!/usr/bin/env python3
"""Run a WASI Preview 1 executable with Wasmtime's Python bindings."""

import sys

import wasmtime


def main():
    if len(sys.argv) < 2:
        return "usage: run_wasi.py MODULE [ARG ...]"

    engine = wasmtime.Engine()
    module = wasmtime.Module.from_file(engine, sys.argv[1])
    store = wasmtime.Store(engine)
    configuration = wasmtime.WasiConfig()
    configuration.argv = sys.argv[1:]
    configuration.inherit_stdin()
    configuration.inherit_stdout()
    configuration.inherit_stderr()
    store.set_wasi(configuration)
    linker = wasmtime.Linker(engine)
    linker.define_wasi()
    instance = linker.instantiate(store, module)
    try:
        instance.exports(store)["_start"](store)
    except wasmtime.ExitTrap as error:
        return error.code
    return 0


if __name__ == "__main__":
    sys.exit(main())
