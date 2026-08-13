#!/usr/bin/env python3
"""The 64-bit block cipher the SQEX container uses.

The published Blowfish algorithm, whose initialization tables are the
hexadecimal expansion of pi's fractional part. The tables are computed
here from that expansion rather than transcribed from anywhere, so this
file carries an algorithm and no copied constants.

One thing here is not the published algorithm: the two halves of a block
are little-endian words, not the big-endian words the specification's test
vectors use. That is established against retail data, not assumed; see
docs/formats/sqex.md.

Shared by tools/make_public_fixtures.py, which enciphers the synthetic
fixtures, and tools/research/census_sqwt.py, which decodes the install.
Neither owns the cipher, so the two cannot disagree about it.
"""

from __future__ import annotations

import struct

BLOCK_SIZE = 8
ROUNDS = 16
MASK = 0xFFFFFFFF


def arctan(x: int, one: int) -> int:
    total = one // x
    term = total
    n = 1
    sign = -1
    square = x * x
    while term:
        term //= square
        n += 2
        total += sign * (term // n)
        sign = -sign
    return total


def pi_hex_words(count: int) -> list[int]:
    """The first `count` 32-bit words of pi's fractional part in hex.

    Machin's formula in integer arithmetic, with guard digits so the last
    word returned is not the one the truncation damages.
    """
    digits = count * 8 + 320
    one = 16**digits
    pi = 16 * arctan(5, one) - 4 * arctan(239, one)
    text = format(pi - 3 * one, "x").rjust(digits, "0")
    return [int(text[index * 8 : index * 8 + 8], 16) for index in range(count)]


_WORDS = pi_hex_words(18 + 1024)
P_INIT = _WORDS[:18]
S_INIT = [_WORDS[18 + box * 256 : 18 + (box + 1) * 256] for box in range(4)]


class Blowfish:
    """A key schedule, expanded once and used for every block of a file."""

    def __init__(self, key: bytes) -> None:
        if not key:
            raise ValueError("the key is the file's own name, and none was supplied")
        self.p = list(P_INIT)
        self.s = [list(box) for box in S_INIT]
        index = 0
        for position in range(18):
            word = 0
            for _ in range(4):
                word = ((word << 8) | key[index % len(key)]) & MASK
                index += 1
            self.p[position] ^= word
        left = right = 0
        for position in range(0, 18, 2):
            left, right = self.encrypt_words(left, right)
            self.p[position], self.p[position + 1] = left, right
        for box in range(4):
            for position in range(0, 256, 2):
                left, right = self.encrypt_words(left, right)
                self.s[box][position], self.s[box][position + 1] = left, right

    def f(self, word: int) -> int:
        a = self.s[0][(word >> 24) & 0xFF]
        b = self.s[1][(word >> 16) & 0xFF]
        c = self.s[2][(word >> 8) & 0xFF]
        d = self.s[3][word & 0xFF]
        return ((((a + b) & MASK) ^ c) + d) & MASK

    def encrypt_words(self, left: int, right: int) -> tuple[int, int]:
        for position in range(ROUNDS):
            left ^= self.p[position]
            right ^= self.f(left)
            left, right = right, left
        left, right = right, left
        right ^= self.p[16]
        left ^= self.p[17]
        return left & MASK, right & MASK

    def decrypt_words(self, left: int, right: int) -> tuple[int, int]:
        for position in range(17, 1, -1):
            left ^= self.p[position]
            right ^= self.f(left)
            left, right = right, left
        left, right = right, left
        right ^= self.p[1]
        left ^= self.p[0]
        return left & MASK, right & MASK

    def _map_blocks(self, body: bytes, step) -> bytes:
        """Every whole block, leaving a shorter final run untouched."""
        out = bytearray(body)
        for index in range(len(body) // BLOCK_SIZE):
            at = index * BLOCK_SIZE
            left, right = struct.unpack_from("<2I", body, at)
            left, right = step(left, right)
            struct.pack_into("<2I", out, at, left, right)
        return bytes(out)

    def decrypt(self, body: bytes) -> bytes:
        return self._map_blocks(body, self.decrypt_words)

    def encrypt(self, body: bytes) -> bytes:
        return self._map_blocks(body, self.encrypt_words)
