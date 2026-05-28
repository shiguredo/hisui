import { fc, test } from "@fast-check/vitest";
import { assert } from "vite-plus/test";
import { isCloseCode, isOfferMessage, isCloseMessage, isServerMessage } from "./types.ts";

const VALID_CLOSE_CODES = [
  "unknown-type",
  "timeout",
  "sdp-error",
  "srd-error",
  "unexpected",
  "missing-sdp",
] as const;

const closeCodeArbitrary = fc.constantFrom(...VALID_CLOSE_CODES);

const offerMessageArbitrary = fc.record({
  type: fc.constant("offer" as const),
  sdp: fc.string(),
});

const closeMessageArbitrary = fc.record({
  type: fc.constant("close" as const),
  code: closeCodeArbitrary,
  reason: fc.string(),
});

const serverMessageArbitrary = fc.oneof(offerMessageArbitrary, closeMessageArbitrary);

test.prop([closeCodeArbitrary])(
  "isCloseCode は有効な CloseCode に対して常に true を返す",
  (code) => {
    assert.isTrue(isCloseCode(code));
  },
);

test.prop([fc.string().filter((s) => !VALID_CLOSE_CODES.includes(s as never))])(
  "isCloseCode は無効な文字列に対して常に false を返す",
  (value) => {
    assert.isFalse(isCloseCode(value));
  },
);

test.prop([offerMessageArbitrary])(
  "isOfferMessage は有効な OfferMessage に対して常に true を返す",
  (message) => {
    assert.isTrue(isOfferMessage(message));
  },
);

test.prop([closeMessageArbitrary])(
  "isCloseMessage は有効な CloseMessage に対して常に true を返す",
  (message) => {
    assert.isTrue(isCloseMessage(message));
  },
);

test.prop([serverMessageArbitrary])(
  "isServerMessage は有効な ServerMessage に対して常に true を返す",
  (message) => {
    assert.isTrue(isServerMessage(message));
  },
);

test.prop([fc.anything()])(
  "isOfferMessage と isCloseMessage が両方 true になることはない",
  (value) => {
    assert.isFalse(isOfferMessage(value) && isCloseMessage(value));
  },
);

test.prop([serverMessageArbitrary])(
  "isServerMessage は isOfferMessage または isCloseMessage と一致する",
  (message) => {
    assert.strictEqual(
      isServerMessage(message),
      isOfferMessage(message) || isCloseMessage(message),
    );
  },
);
