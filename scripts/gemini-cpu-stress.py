"""Bounded, opt-in laptop CPU load for manual microphone continuity testing.

Run alongside Handy Gemini and a known 60-second passage. This deliberately
contains no audio/clipboard/key access: a human compares the pasted transcript.
"""
import argparse
import multiprocessing as mp
import os
import time


def burn(until):
    value = 1
    while time.monotonic() < until:
        for _ in range(10000):
            value = (value * 1664525 + 1013904223) & 0xFFFFFFFF


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, default=60, choices=range(1, 121))
    args = parser.parse_args()
    cpus = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else (os.cpu_count() or 1)
    end = time.monotonic() + args.seconds
    workers = [mp.Process(target=burn, args=(end,)) for _ in range(cpus)]
    print(f"Loading {cpus} available cores for {args.seconds} seconds. Dictate the known passage now.", flush=True)
    try:
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(timeout=max(0, end - time.monotonic()) + 2)
    except KeyboardInterrupt:
        pass
    finally:
        for worker in workers:
            if worker.is_alive():
                worker.terminate()
            if worker.pid is not None:
                worker.join(timeout=2)
    print("CPU load ended. Compare the complete pasted text with the reference passage.")


if __name__ == "__main__":
    main()
