import json, socket, sys, os
SOCK = os.environ.get("HERDR_SOCKET_PATH", os.path.expanduser("~/.config/herdr/herdr.sock"))
def rpc(method, params=None, sock_path=SOCK, timeout=20):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock_path)
    s.sendall((json.dumps({"id": "kampr-probe", "method": method, "params": params or {}}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        c = s.recv(65536)
        if not c: break
        buf += c
    s.close()
    return json.loads(buf.decode().splitlines()[0]) if buf.strip() else None
if __name__ == "__main__":
    print(json.dumps(rpc(sys.argv[1], json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}), indent=1))
