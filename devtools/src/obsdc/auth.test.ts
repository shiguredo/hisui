import { test, assert } from "vite-plus/test";
import { generateAuthenticationString } from "./auth.ts";

test("generateAuthenticationString は仕様通りの認証文字列を生成する", async () => {
  // プロトコル仕様のサンプル値
  const password = "supersecretpassword";
  const salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
  const challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";

  const result = await generateAuthenticationString(password, salt, challenge);

  // 結果は Base64 文字列であること
  assert.match(result, /^[A-Za-z0-9+/]+=*$/);
  // 長さは SHA-256 の Base64 エンコード (44 文字)
  assert.strictEqual(result.length, 44);
});

test("generateAuthenticationString は同じ入力に対して同じ結果を返す", async () => {
  const password = "testpassword";
  const salt = "testsalt";
  const challenge = "testchallenge";

  const result1 = await generateAuthenticationString(password, salt, challenge);
  const result2 = await generateAuthenticationString(password, salt, challenge);

  assert.strictEqual(result1, result2);
});

test("generateAuthenticationString はパスワードが異なれば異なる結果を返す", async () => {
  const salt = "testsalt";
  const challenge = "testchallenge";

  const result1 = await generateAuthenticationString("password1", salt, challenge);
  const result2 = await generateAuthenticationString("password2", salt, challenge);

  assert.notStrictEqual(result1, result2);
});
