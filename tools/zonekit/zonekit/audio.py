"""Modern .scd sounds -> 1.23b sound files.

Music and zone ambience: the Ogg Vorbis stream of a modern version-3 file is
decoded and written onto a stock version-2 1.23b file. Spot sounds: one mono
MS-ADPCM entry is written onto a stock one-entry 1.23b file.

Version 3 obfuscation follows Physis (GPL-3.0, https://github.com/redstrate/Physis) src/scd.rs
(attribution for translated behaviour).
"""

import struct

XOR = bytes.fromhex(
    "3A323232037E12F7B2E2A267323222323252161B3CA1547B1B97A6931A4BAAA6"
    "7A7B1B97A6F702BBAAA6BBF72A51BE03F42A51BE03F42A51BE12065627323236"
    "32B21A3BBC91D47B58FC0B552A15BC40920B5B7C0A951235B863D20B3BF0C714"
    "515C948694595CFC1B173A3F6B3732323032727A13B726607A13B72650BA13B4"
    "2A50BA13B52E40FA1395AE4038189A92B03800FA12B17E00DB96A17C08DB9A91"
    "BC08D81A86E270391F86E0787E03E764519C8F346F4E41FC0BD5AE41FC0BD5AE"
    "41FC3B70716433321232323670342B5622703A13B72660BA1B94AA403800FAB2"
    "E2A26732321232B23232323275A3267B8326F9832EFFE3167DC01E632107E301"
)
assert len(XOR) == 256 and XOR[-4:] == bytes([0x21, 0x07, 0xE3, 0x01])


class AudioError(Exception):
    pass


def _check(cond, msg):
    if not cond:
        raise AudioError(msg)


def _audio_entries(d):
    _check(d[:8] == b"SEDBSSCF", "not an SSCF sound file")
    h = struct.unpack_from("<H", d, 0x0E)[0]
    n = struct.unpack_from("<H", d, h + 4)[0]
    o = struct.unpack_from("<I", d, h + 12)[0]
    return [struct.unpack_from("<I", d, o + i * 4)[0] for i in range(n)]


# ---- music / ambience (Vorbis) -------------------------------------------------------------------------------


def read_vorbis(d):
    """Decode the one Vorbis entry of an SSCF v3 file (bytes); returns a dict with the plain Ogg stream."""
    _check(
        d[:8] == b"SEDBSSCF" and struct.unpack_from("<I", d, 8)[0] == 3,
        "not an SSCF v3 file",
    )
    offs = _audio_entries(d)
    _check(len(offs) == 1, "expected one audio entry, found %d" % len(offs))
    e = offs[0]
    size, ch, rate, codec, loop_s, loop_e, extra, aux = struct.unpack_from(
        "<IIIiIIII", d, e
    )
    _check(codec == 6, "codec %d is not Vorbis" % codec)
    x = e + 0x20
    ver, ssize, xorb = d[x], d[x + 1], d[x + 2]
    seek_size, hdr_size = struct.unpack_from("<II", d, x + 0x10)
    _check(
        ssize == 0x20 and 0x20 + seek_size + hdr_size == extra,
        "extra block does not add up",
    )
    s = x + 0x20 + seek_size
    body = bytearray(d[s : s + hdr_size + size])
    _check(len(body) == hdr_size + size, "file is short")
    if ver == 2:
        for i in range(hdr_size):
            body[i] ^= xorb
    elif ver == 3:
        b1 = size & 0x7F
        b2 = b1 & 0x3F
        tab = bytes(XOR[(b2 + i) & 0xFF] ^ b1 for i in range(256))
        n = len(body)
        full = tab * (n // 256 + 1)
        body = bytearray(
            (
                int.from_bytes(body, "little") ^ int.from_bytes(full[:n], "little")
            ).to_bytes(n, "little")
        )
    else:
        raise AudioError("unknown Vorbis block version %d" % ver)
    return dict(
        ogg=bytes(body),
        header_size=hdr_size,
        size=size,
        channels=ch,
        rate=rate,
        loop=(loop_s, loop_e),
        version=ver,
        entry=e,
        aux=aux,
        seek_header=bytes(d[x : x + 0x20]),
        seek_table=bytes(d[x + 0x20 : s]),
        xor_byte=xorb,
    )


def ogg_pages(ogg):
    """List (offset, length, granule) of every Ogg page; the chain must cover the stream exactly."""
    out, o = [], 0
    while o < len(ogg):
        _check(ogg[o : o + 4] == b"OggS", "no Ogg page at %#x" % o)
        nseg = ogg[o + 26]
        ln = 27 + nseg + sum(ogg[o + 27 : o + 27 + nseg])
        out.append((o, ln, struct.unpack_from("<q", ogg, o + 6)[0]))
        o += ln
    _check(o == len(ogg), "trailing bytes after the last Ogg page")
    return out


def bgm_to_1x(modern, template):
    """modern: bytes of a modern music/ambience .scd; template: bytes of a stock 1.0 version-2 file. Returns bytes."""
    m = read_vorbis(modern)
    ogg_pages(m["ogg"])
    tm = read_vorbis(template)
    _check(tm["version"] == 2, "template must be a version-2 file of the 1.0 client")
    e, hs, ogg, key = tm["entry"], m["header_size"], m["ogg"], m["xor_byte"]
    hdr = bytes(b ^ key for b in ogg[:hs])
    sh = bytearray(m["seek_header"])
    sh[0] = 2
    sh[3] = 0  # byte 3 is 0 in every stock version-2 track
    extra = 0x20 + len(m["seek_table"]) + hs
    entry = struct.pack(
        "<IIIiIIII",
        m["size"],
        m["channels"],
        m["rate"],
        6,
        m["loop"][0],
        m["loop"][1],
        extra,
        tm["aux"],
    )
    body = bytearray(template[:e]) + entry + sh + m["seek_table"] + hdr + ogg[hs:]
    body += b"\0" * (-len(body) % 16)
    struct.pack_into("<I", body, 0x10, len(body))
    back = read_vorbis(bytes(body))
    _check(
        back["ogg"] == ogg and back["loop"] == m["loop"] and back["version"] == 2,
        "read-back differs",
    )
    return bytes(body)


def bgm_info(d):
    m = read_vorbis(d)
    pg = ogg_pages(m["ogg"])
    starts = {o for o, _, _ in pg}
    return (
        "Vorbis, version %d, %d channels, %d Hz, header %d B, audio %d B, %d pages, %.1f s, loop %d..%d "
        "(loop start on a page: %s, loop end = end: %s)"
        % (
            m["version"],
            m["channels"],
            m["rate"],
            m["header_size"],
            m["size"],
            len(pg),
            pg[-1][2] / m["rate"],
            m["loop"][0],
            m["loop"][1],
            (m["loop"][0] + m["header_size"]) in starts,
            m["loop"][1] == m["size"],
        )
    )


# ---- spot sounds (MS-ADPCM) ----------------------------------------------------------------------------------


def adpcm_entries(d):
    out = []
    for e in _audio_entries(d):
        size, ch, rate, codec, ls, le, extra, aux = struct.unpack_from(
            "<IIIiIIII", d, e
        )
        out.append(
            dict(
                off=e,
                size=size,
                ch=ch,
                rate=rate,
                codec=codec,
                loop=(ls, le),
                extra=extra,
                aux=aux,
                fmt=d[e + 0x20 : e + 0x20 + 50],
                data=d[e + 0x20 + extra : e + 0x20 + extra + size],
            )
        )
    return out


def _check_adpcm(a):
    _check(a["codec"] == 12 and a["ch"] == 1, "entry is not mono MS-ADPCM")
    tag, ch, _, _, align, _, cb = struct.unpack_from("<HHIIHHH", a["fmt"], 0)
    _check(
        tag == 2
        and ch == 1
        and cb == 32
        and len(a["data"]) == a["size"]
        and a["size"] % align == 0,
        "unexpected format block",
    )
    return align, struct.unpack_from("<H", a["fmt"], 18)[0]


def se_to_1x(modern, index, template):
    """Entry `index` of a modern spot-sound .scd onto a stock one-entry 1.0 file. Returns bytes."""
    ents = adpcm_entries(modern)
    _check(0 <= index < len(ents), "entry %d not in 0..%d" % (index, len(ents) - 1))
    a = ents[index]
    layout = _check_adpcm(a)
    te = adpcm_entries(template)
    _check(len(te) == 1, "template must have exactly one audio entry")
    ta = te[0]
    _check(
        _check_adpcm(ta) == layout and ta["extra"] >= 50,
        "template block layout differs",
    )
    e = ta["off"]
    pad = template[e + 0x20 + 50 : e + 0x20 + ta["extra"]]
    body = (
        bytearray(template[:e])
        + struct.pack(
            "<IIIiIIII",
            a["size"],
            1,
            a["rate"],
            12,
            a["loop"][0],
            a["loop"][1],
            ta["extra"],
            ta["aux"],
        )
        + a["fmt"]
        + pad
        + a["data"]
    )
    body += b"\0" * (
        len(template) - (e + 0x20 + ta["extra"] + ta["size"])
    )  # the template's own tail padding
    struct.pack_into("<I", body, 0x10, len(body))
    back = adpcm_entries(bytes(body))
    _check(
        len(back) == 1
        and back[0]["data"] == a["data"]
        and back[0]["loop"] == a["loop"]
        and back[0]["fmt"] == a["fmt"],
        "read-back differs",
    )
    return bytes(body)


def se_wav(modern, index):
    a = adpcm_entries(modern)[index]
    align, spb = _check_adpcm(a)
    n = a["size"] // align * spb
    body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", 50)
        + a["fmt"]
        + b"fact"
        + struct.pack("<II", 4, n)
        + b"data"
        + struct.pack("<I", a["size"])
        + a["data"]
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def info(d):
    """One line per audio entry; Vorbis files get the full music summary."""
    ents = adpcm_entries(d)
    if len(ents) == 1 and ents[0]["codec"] == 6:
        return [bgm_info(d)]
    return [
        "entry %d at %#x: codec %d, %d ch, %d Hz, %d B, loop %d..%d, extra %d"
        % (
            i,
            a["off"],
            a["codec"],
            a["ch"],
            a["rate"],
            a["size"],
            a["loop"][0],
            a["loop"][1],
            a["extra"],
        )
        for i, a in enumerate(ents)
    ]
