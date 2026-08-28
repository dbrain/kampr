import re, sys, unicodedata

class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.g = [[' ']*cols for _ in range(rows)]
        self.x = self.y = 0
    def put(self, ch):
        if self.y >= self.rows or self.x >= self.cols: return
        w = 2 if unicodedata.east_asian_width(ch) in ('W','F') else 1
        self.g[self.y][self.x] = ch
        if w == 2 and self.x+1 < self.cols: self.g[self.y][self.x+1] = ''
        self.x += w
    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.g)

CSI = re.compile(r'\x1b\[([0-9;?]*)([ -/]*)([@-~])')
OSC = re.compile(r'\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)')
DCS = re.compile(r'\x1b[PX^_].*?(?:\x1b\\|\x07)', re.S)

def feed(sc, data):
    i = 0
    n = len(data)
    while i < n:
        c = data[i]
        if c == '\x1b':
            m = OSC.match(data, i) or DCS.match(data, i)
            if m: i = m.end(); continue
            m = CSI.match(data, i)
            if m:
                params, inter, fin = m.group(1), m.group(2), m.group(3)
                ps = [int(p) if p.isdigit() else 0 for p in params.lstrip('?').split(';')] if params.lstrip('?') else []
                def p(k, d=1):
                    return ps[k] if k < len(ps) and ps[k] else d
                if fin == 'H' or fin == 'f':
                    sc.y = p(0)-1; sc.x = p(1)-1
                elif fin == 'A': sc.y = max(0, sc.y-p(0))
                elif fin == 'B': sc.y = min(sc.rows-1, sc.y+p(0))
                elif fin == 'C': sc.x = min(sc.cols-1, sc.x+p(0))
                elif fin == 'D': sc.x = max(0, sc.x-p(0))
                elif fin == 'G': sc.x = p(0)-1
                elif fin == 'd': sc.y = p(0)-1
                elif fin == 'J':
                    mode = ps[0] if ps else 0
                    if mode == 2 or mode == 3:
                        sc.g = [[' ']*sc.cols for _ in range(sc.rows)]
                    elif mode == 0:
                        for xx in range(sc.x, sc.cols): sc.g[sc.y][xx]=' '
                        for yy in range(sc.y+1, sc.rows): sc.g[yy]=[' ']*sc.cols
                    else:
                        for yy in range(0, sc.y): sc.g[yy]=[' ']*sc.cols
                        for xx in range(0, min(sc.x+1,sc.cols)): sc.g[sc.y][xx]=' '
                elif fin == 'K':
                    mode = ps[0] if ps else 0
                    if mode == 0:
                        for xx in range(sc.x, sc.cols): sc.g[sc.y][xx]=' '
                    elif mode == 1:
                        for xx in range(0, min(sc.x+1,sc.cols)): sc.g[sc.y][xx]=' '
                    else: sc.g[sc.y]=[' ']*sc.cols
                i = m.end(); continue
            i += 2; continue
        if c == '\r': sc.x = 0; i+=1; continue
        if c == '\n':
            sc.y = min(sc.rows-1, sc.y+1); i+=1; continue
        if c == '\b': sc.x = max(0, sc.x-1); i+=1; continue
        if c == '\t': sc.x = min(sc.cols-1, (sc.x//8+1)*8); i+=1; continue
        if ord(c) < 32: i+=1; continue
        sc.put(c); i+=1
    return sc
