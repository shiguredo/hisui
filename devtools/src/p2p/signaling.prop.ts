import { fc, test } from "@fast-check/vitest";
import { assert } from "vite-plus/test";
import { parseServerMessage, serializeClientMessage } from "./signaling.ts";

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

const answerMessageArbitrary = fc.record({
  type: fc.constant("answer" as const),
  sdp: fc.string(),
});

const disconnectMessageArbitrary = fc.constant({
  type: "disconnect" as const,
});

const clientMessageArbitrary = fc.oneof(answerMessageArbitrary, disconnectMessageArbitrary);

test.prop([serverMessageArbitrary])(
  "parseServerMessage はシリアライズされた ServerMessage をラウンドトリップできる",
  (message) => {
    const serialized = JSON.stringify(message);
    const parsed = parseServerMessage(serialized);
    assert.deepStrictEqual(parsed, message);
  },
);

test.prop([clientMessageArbitrary])(
  "serializeClientMessage の結果は有効な JSON である",
  (message) => {
    const serialized = serializeClientMessage(message);
    const parsed = JSON.parse(serialized);
    assert.deepStrictEqual(parsed, message);
  },
);

test.prop([
  fc.string().filter((s) => {
    try {
      JSON.parse(s);
      return false;
    } catch {
      return true;
    }
  }),
])("parseServerMessage は不正な JSON で常にエラーを投げる", (invalidJson) => {
  assert.throws(
    () => parseServerMessage(invalidJson),
    /failed to parse server message: invalid JSON/,
  );
});
