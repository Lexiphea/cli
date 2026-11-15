import json
import os
import socket
from typing import Any

socket_base = f"{os.getenv('XDG_RUNTIME_DIR')}/hypr/{os.getenv('HYPRLAND_INSTANCE_SIGNATURE')}"
socket_path = f"{socket_base}/.socket.sock"


def message(msg: str, is_json: bool = True) -> str | dict[str, Any]:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(socket_path)

        if is_json:
            msg = f"j/{msg}"
        sock.send(msg.encode())

        resp = sock.recv(8192).decode()
        while True:
            new_resp = sock.recv(8192)
            if not new_resp:
                break
            resp += new_resp.decode()

        return json.loads(resp) if is_json else resp
