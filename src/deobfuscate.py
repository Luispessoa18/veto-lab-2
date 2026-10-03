"""Decodifica ofuscações comuns antes da detecção de prompt injection.

Gera variantes legíveis de um texto (base64, hex, rot13, invertido, homóglifos,
leetspeak, caracteres invisíveis) para que regras e Prompt Guard analisem o
conteúdo real, e não só a forma codificada.
"""
import base64
import binascii
import codecs
import re
import unicodedata

ZERO_WIDTH = dict.fromkeys(map(ord, "​‌‍⁠﻿­"), None)
HOMOGLYPHS = str.maketrans({
    # cirílico
    "а": "a", "е": "e", "о": "o", "р": "p", "с": "c", "у": "y", "х": "x", "і": "i", "ј": "j",
    "ѕ": "s", "ԁ": "d", "һ": "h", "ӏ": "l", "ԛ": "q", "ԝ": "w", "т": "t", "к": "k", "м": "m",
    "н": "h", "в": "b", "А": "A", "В": "B", "Е": "E", "К": "K", "М": "M", "Н": "H", "О": "O",
    "Р": "P", "С": "C", "Т": "T", "Х": "X", "У": "Y", "І": "I",
    # grego
    "α": "a", "ο": "o", "ε": "e", "ρ": "p", "ν": "v", "τ": "t", "ι": "i", "κ": "k", "υ": "u",
    "Α": "A", "Β": "B", "Ε": "E", "Ι": "I", "Κ": "K", "Μ": "M", "Ν": "N", "Ο": "O", "Ρ": "P", "Τ": "T",
})
LEET = str.maketrans({"0": "o", "1": "i", "3": "e", "4": "a", "5": "s", "6": "g", "7": "t", "@": "a", "$": "s"})
LEET_WORD = re.compile(r"(?<![\w@$])(?=[\w@$]*[a-zA-Z])(?=[\w@$]*[0-9@$])[\w@$]{3,20}(?![\w@$])")
BASE64 = re.compile(r"(?<![A-Za-z0-9+/=])[A-Za-z0-9+/]{16,}={0,2}(?![A-Za-z0-9+/=])")
HEX = re.compile(r"(?:\b|0x)((?:[0-9a-fA-F]{2}[\s:]?){8,})")
COMMON_WORDS = frozenset("""the and to you of a in is it that this for your me be are not do with on as all
have been say print tell what please i ignore previous instructions system prompt secret password reveal
output write now instead rules forget disregard approve allow transaction from will can my so if or""".split())
MAX_LEN = 12000


def english_score(text):
    words = re.findall(r"[a-zA-Z]+", text.lower())
    return sum(word in COMMON_WORDS for word in words) / len(words) if words else 0.0


def printable(text):
    return bool(text) and sum(ch.isprintable() or ch in "\n\t" for ch in text) / len(text) > 0.95


def unleet(word):
    """Desfaz leetspeak só em palavras com maioria de letras (preserva versões, hex e endereços)."""
    letters = sum(ch.isalpha() for ch in word)
    if word.lower().startswith("0x") or letters < len(word) * 0.5:
        return word
    return word.translate(LEET)


def normalize(text):
    """NFKC, remove invisíveis, troca homóglifos e desfaz leetspeak em palavras mistas."""
    clean = unicodedata.normalize("NFKC", text).translate(ZERO_WIDTH).translate(HOMOGLYPHS)
    return LEET_WORD.sub(lambda match: unleet(match.group(0)), clean)


def decode_variants(text):
    """Lista de (método, texto decodificado) diferentes do original."""
    text = str(text)[:MAX_LEN]
    variants = []
    normalized = normalize(text)
    if normalized != text:
        variants.append(("normalize", normalized))
    for match in BASE64.finditer(text):
        try:
            decoded = base64.b64decode(match.group(0) + "=" * (-len(match.group(0)) % 4)).decode("utf-8")
        except (binascii.Error, UnicodeDecodeError, ValueError):
            continue
        if printable(decoded):
            variants.append(("base64", decoded))
    for match in HEX.finditer(text):
        try:
            decoded = bytes.fromhex(re.sub(r"[\s:]", "", match.group(1))).decode("utf-8")
        except (ValueError, UnicodeDecodeError):
            continue
        if printable(decoded):
            variants.append(("hex", decoded))
    base_score = english_score(normalized)
    for method, candidate in (("rot13", codecs.decode(normalized, "rot13")), ("reversed", normalized[::-1])):
        hits = sum(word in COMMON_WORDS for word in re.findall(r"[a-zA-Z]+", candidate.lower()))
        if hits >= 3 and english_score(candidate) >= max(0.2, base_score + 0.15):
            variants.append((method, candidate))
    seen, unique = {text}, []
    for method, candidate in variants:
        if candidate not in seen:
            seen.add(candidate)
            unique.append((method, candidate))
    return unique


def all_forms(text):
    """O texto original seguido das variantes decodificadas."""
    return [str(text)[:MAX_LEN], *(candidate for _, candidate in decode_variants(text))]
