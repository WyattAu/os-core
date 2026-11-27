#!/usr/bin/env python3
"""UDP evidence collector for omniosk hardware runs.

Binds 192.168.1.1:9933 (the operator PC's LAN address) and appends
every received datagram to /tmp/opencode/omniosk-udp.log with a
timestamp and the sender address. Run in the background before
rebooting the target into OMNIOSK.
"""

import datetime
import socket

LISTEN = ("", 9933)  # wildcard: also catches frames with a wrong dst IP
LOG = "/tmp/opencode/omniosk-udp.log"


def main() -> None:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(LISTEN)
    line = f"{datetime.datetime.now().isoformat()} collector listening on {LISTEN[0]}:{LISTEN[1]}"
    print(line, flush=True)
    def log_line(entry: str) -> None:
        # Reopen per write: a log rotated or removed between runs must
        # not strand this process writing into a deleted inode (that
        # cost one hardware cycle's beacon capture).
        with open(LOG, "a") as log:
            log.write(entry + "\n")

    log_line(line)

    while True:
        data, addr = s.recvfrom(2048)
        entry = (
            f"{datetime.datetime.now().isoformat()} "
            f"{addr[0]}:{addr[1]} {len(data)}B {data.decode(errors='replace')}"
        )
        print(entry, flush=True)
        log_line(entry)


if __name__ == "__main__":
    main()
