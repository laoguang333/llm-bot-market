"""Build release binaries within the platform CPU budget.

--native enables instructions for this CPU and writes to target/native/release.
Native binaries require a compatible CPU; ordinary release builds stay portable.
"""

import argparse
import os
from pathlib import Path
import subprocess


def windows_build_budget():
    # The build process and its children share the same affinity mask. This also
    # bounds LLVM/linker worker pools, not just Cargo's number of rustc jobs.
    import ctypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetCurrentProcess.restype = ctypes.c_void_p
    kernel.GetProcessAffinityMask.argtypes = [
        ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)
    ]
    kernel.SetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    process = kernel.GetCurrentProcess()
    available, system = ctypes.c_size_t(), ctypes.c_size_t()
    if not kernel.GetProcessAffinityMask(process, ctypes.byref(available), ctypes.byref(system)):
        raise ctypes.WinError(ctypes.get_last_error())
    bits = [1 << bit for bit in range(ctypes.sizeof(ctypes.c_size_t) * 8)
            if available.value & (1 << bit)]
    budget = min(len(bits), max(1, (os.cpu_count() or 1) // 2))
    if not kernel.SetProcessAffinityMask(process, sum(bits[:budget])):
        raise ctypes.WinError(ctypes.get_last_error())
    return budget


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    env = os.environ.copy()
    command = ["cargo", "build", "--release"]
    if os.name == "nt":
        budget = windows_build_budget()
        command += ["--jobs", str(budget)]
        print(f"Windows build CPU budget: {budget}", flush=True)
    output = root / "target"
    if args.native:
        if "CARGO_ENCODED_RUSTFLAGS" in env:
            flags = env["CARGO_ENCODED_RUSTFLAGS"]
            env["CARGO_ENCODED_RUSTFLAGS"] = (flags + "\x1f" if flags else "") + "-C\x1ftarget-cpu=native"
        else:
            env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-cpu=native").strip()
        output /= "native"
        print("Native build: only run these binaries on compatible CPUs.", flush=True)
    command += ["--target-dir", str(output)]
    subprocess.run(command, cwd=root, env=env, check=True)
    print(f"Binaries: {output / 'release'}", flush=True)


if __name__ == "__main__":
    main()
