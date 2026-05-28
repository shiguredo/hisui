import { test, assert } from "vite-plus/test";
import { isCloseCode, isOfferMessage, isCloseMessage, isServerMessage } from "./types.ts";

// isCloseCode

test("isCloseCode は有効なコードに対して true を返す", () => {
  const validCodes = [
    "unknown-type",
    "timeout",
    "sdp-error",
    "srd-error",
    "unexpected",
    "missing-sdp",
  ];
  for (const code of validCodes) {
    assert.isTrue(isCloseCode(code), `${code} は有効な CloseCode`);
  }
});

test("isCloseCode は無効な値に対して false を返す", () => {
  assert.isFalse(isCloseCode("invalid"));
  assert.isFalse(isCloseCode(""));
  assert.isFalse(isCloseCode(null));
  assert.isFalse(isCloseCode(42));
  assert.isFalse(isCloseCode({}));
});

// isOfferMessage

test("isOfferMessage は有効な offer メッセージに対して true を返す", () => {
  assert.isTrue(isOfferMessage({ type: "offer", sdp: "v=0\r\n" }));
});

test("isOfferMessage は sdp が空文字列でも true を返す", () => {
  assert.isTrue(isOfferMessage({ type: "offer", sdp: "" }));
});

test("isOfferMessage は type が異なる場合 false を返す", () => {
  assert.isFalse(isOfferMessage({ type: "answer", sdp: "v=0\r\n" }));
});

test("isOfferMessage は sdp がない場合 false を返す", () => {
  assert.isFalse(isOfferMessage({ type: "offer" }));
});

test("isOfferMessage は sdp が文字列でない場合 false を返す", () => {
  assert.isFalse(isOfferMessage({ type: "offer", sdp: 42 }));
});

test("isOfferMessage は null に対して false を返す", () => {
  assert.isFalse(isOfferMessage(null));
});

test("isOfferMessage は文字列に対して false を返す", () => {
  assert.isFalse(isOfferMessage("offer"));
});

// isCloseMessage

test("isCloseMessage は有効な close メッセージに対して true を返す", () => {
  assert.isTrue(isCloseMessage({ type: "close", code: "timeout", reason: "timed out" }));
});

test("isCloseMessage は無効な code に対して false を返す", () => {
  assert.isFalse(isCloseMessage({ type: "close", code: "invalid", reason: "test" }));
});

test("isCloseMessage は reason がない場合 false を返す", () => {
  assert.isFalse(isCloseMessage({ type: "close", code: "timeout" }));
});

test("isCloseMessage は reason が文字列でない場合 false を返す", () => {
  assert.isFalse(isCloseMessage({ type: "close", code: "timeout", reason: 42 }));
});

test("isCloseMessage は type が異なる場合 false を返す", () => {
  assert.isFalse(isCloseMessage({ type: "offer", code: "timeout", reason: "test" }));
});

test("isCloseMessage は null に対して false を返す", () => {
  assert.isFalse(isCloseMessage(null));
});

// isServerMessage

test("isServerMessage は有効な offer メッセージに対して true を返す", () => {
  assert.isTrue(isServerMessage({ type: "offer", sdp: "v=0\r\n" }));
});

test("isServerMessage は有効な close メッセージに対して true を返す", () => {
  assert.isTrue(isServerMessage({ type: "close", code: "timeout", reason: "timed out" }));
});

test("isServerMessage は無効なメッセージに対して false を返す", () => {
  assert.isFalse(isServerMessage({ type: "answer", sdp: "v=0\r\n" }));
  assert.isFalse(isServerMessage(null));
  assert.isFalse(isServerMessage("string"));
  assert.isFalse(isServerMessage(42));
});
