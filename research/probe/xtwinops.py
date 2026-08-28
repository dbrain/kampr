import os, sys, time, json, termios, tty, fcntl, struct, select, signal

OUT = sys.argv[1]
NAME = sys.argv[2]

tty_fd = os.open("/dev/tty", os.O_RDWR)

def size():
    r, c, xp, yp = struct.unpack("HHHH", fcntl.ioctl(tty_fd, termios.TIOCGWINSZ, b"\0"*8))
    return [r, c, xp, yp]

winch = []
signal.signal(signal.SIGWINCH, lambda s, f: winch.append([time.time(), size()]))

old = termios.tcgetattr(tty_fd)
tty.setraw(tty_fd)

def w(s): os.write(tty_fd, s.encode())

def q(seq, secs=0.6):
    while select.select([tty_fd],[],[],0.0)[0]:
        os.read(tty_fd, 4096)
    w(seq)
    buf = b""; t0 = time.time()
    while time.time()-t0 < secs:
        r,_,_ = select.select([tty_fd],[],[],0.1)
        if r: buf += os.read(tty_fd, 4096)
        if buf.endswith(b"t") or buf.endswith(b"\x1b\\") or buf.endswith(b"c"): break
    return buf.decode("latin1")

res = {"name": NAME, "term": os.environ.get("TERM"), "term_program": os.environ.get("TERM_PROGRAM"),
       "term_program_version": os.environ.get("TERM_PROGRAM_VERSION"),
       "start": size()}

res["xtversion_csi>0q"] = q("\x1b[>0q")
res["da1_csi_c"] = q("\x1b[c")
res["csi_18_t"] = q("\x1b[18t")
res["csi_14_t"] = q("\x1b[14t")
res["csi_16_t"] = q("\x1b[16t")
res["csi_15_t"] = q("\x1b[15t")

def resize_test(rows, cols, label, budget=3.0):
    before = size()
    nw = len(winch)
    t0 = time.time()
    w(f"\x1b[8;{rows};{cols}t")
    landed = None
    while time.time()-t0 < budget:
        s = size()
        if s[0] != before[0] or s[1] != before[1]:
            landed = time.time()-t0
            break
        time.sleep(0.005)
    time.sleep(0.35)
    after = size()
    return {"label": label, "asked": [rows, cols], "before": before, "after": after,
            "first_change_ms": None if landed is None else round(landed*1000, 1),
            "winch_delta": len(winch)-nw, "settled_18t": q("\x1b[18t"),
            "settled_14t": q("\x1b[14t")}

start = res["start"]
res["grow"]   = resize_test(30, 100, "grow to 30x100")
res["shrink"] = resize_test(20, 60,  "shrink to 20x60")
res["huge"]   = resize_test(400, 900, "ask 400x900 (clamp?)")
res["rows_only_semantics"] = resize_test(24, 80, "back to 24x80")
res["restore"] = resize_test(start[0], start[1], "restore original")
res["winch_log"] = [[round(t-0,3), s] for t, s in winch][-12:]
res["winch_total"] = len(winch)

termios.tcsetattr(tty_fd, termios.TCSANOW, old)
open(OUT, "w").write(json.dumps(res, indent=1))
