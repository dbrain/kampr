import base64, os, socket, struct, time

class WS:
    """A blocking RFC6455 client, enough for the node's wire: text in, text out, pongs answered."""

    def __init__(self, host, port, path="/ws", protocol=None, timeout=20):
        self.sock = socket.create_connection((host, port), timeout)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        key = base64.b64encode(os.urandom(16)).decode()
        head = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        if protocol:
            head.append(f"Sec-WebSocket-Protocol: {protocol}")
        self.sock.sendall(("\r\n".join(head) + "\r\n\r\n").encode())
        self.buf = b""
        while b"\r\n\r\n" not in self.buf:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("upgrade closed")
            self.buf += chunk
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        if b"101" not in head.split(b"\r\n")[0]:
            raise RuntimeError(head.decode(errors="replace"))

    def fileno(self):
        return self.sock.fileno()

    def send(self, text, opcode=0x1):
        payload = text.encode() if isinstance(text, str) else text
        n = len(payload)
        header = bytes([0x80 | opcode])
        if n < 126:
            header += bytes([0x80 | n])
        elif n < 65536:
            header += bytes([0x80 | 126]) + struct.pack("!H", n)
        else:
            header += bytes([0x80 | 127]) + struct.pack("!Q", n)
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(header + mask + masked)

    def _fill(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("closed")
            self.buf += chunk

    def frame(self):
        """One frame, blocking. Returns (opcode, payload); pings are answered here."""
        while True:
            self._fill(2)
            b0, b1 = self.buf[0], self.buf[1]
            opcode = b0 & 0x0F
            n = b1 & 0x7F
            off = 2
            if n == 126:
                self._fill(4)
                n = struct.unpack("!H", self.buf[2:4])[0]
                off = 4
            elif n == 127:
                self._fill(10)
                n = struct.unpack("!Q", self.buf[2:10])[0]
                off = 10
            self._fill(off + n)
            payload = self.buf[off : off + n]
            self.buf = self.buf[off + n :]
            if opcode == 0x9:
                self.send(payload, opcode=0xA)
                continue
            return opcode, payload

    def pending(self):
        return len(self.buf) >= 2

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass
