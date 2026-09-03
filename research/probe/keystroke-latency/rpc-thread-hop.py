"""What a scheduler hop between connect and write costs on herdr's socket.

`tokio::net::UnixStream::connect(..).await` completes on the reactor, so the write that follows it
runs only once a worker has been woken and scheduled. That hop is the window herdr's first look
lands in. Modelled here by doing the write on a second thread the connecting thread wakes.
"""
import json, os, socket, threading, time

SOCK = os.environ["HERDR_SOCKET_PATH"]
N = int(os.environ.get("N", "150"))
REQ = (json.dumps({"id": "p", "method": "pane.list", "params": {}}) + "\n").encode()


def summarise(label, took):
    took.sort()
    def q(p): return took[round((len(took) - 1) * p)]
    print(f"{label}: n={len(took)} min {took[0]:.2f}  p50 {q(.5):.2f}  p90 {q(.9):.2f}  "
          f"max {took[-1]:.2f} ms  over-50ms {sum(1 for v in took if v > 50)} "
          f"({100 * sum(1 for v in took if v > 50) / len(took):.0f}%)")


def same_thread():
    took = []
    for _ in range(N):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(20)
        t = time.perf_counter()
        s.connect(SOCK)
        s.sendall(REQ)
        buf = b""
        while b"\n" not in buf:
            c = s.recv(65536)
            if not c:
                break
            buf += c
        took.append((time.perf_counter() - t) * 1000)
        s.close()
        time.sleep(0.05)
    return took


def across_a_hop():
    took = []
    ready = threading.Semaphore(0)
    done = threading.Semaphore(0)
    box = {}

    def writer():
        while True:
            ready.acquire()
            if box.get("stop"):
                return
            box["sock"].sendall(REQ)
            done.release()

    threading.Thread(target=writer, daemon=True).start()
    for _ in range(N):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(20)
        t = time.perf_counter()
        s.connect(SOCK)
        box["sock"] = s
        ready.release()
        done.acquire()
        buf = b""
        while b"\n" not in buf:
            c = s.recv(65536)
            if not c:
                break
            buf += c
        took.append((time.perf_counter() - t) * 1000)
        s.close()
        time.sleep(0.05)
    box["stop"] = True
    ready.release()
    return took


summarise("connect and write on one thread ", same_thread())
summarise("write across a thread wake-up   ", across_a_hop())
summarise("connect and write on one thread ", same_thread())
