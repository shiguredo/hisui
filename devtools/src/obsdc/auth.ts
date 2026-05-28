// OBS WebSocket 5.x SHA-256 認証

export async function generateAuthenticationString(
  password: string,
  salt: string,
  challenge: string,
): Promise<string> {
  const encoder = new TextEncoder();

  // base64_secret = base64(sha256(password + salt))
  const passwordSaltHash = await crypto.subtle.digest("SHA-256", encoder.encode(password + salt));
  const base64Secret = btoa(String.fromCodePoint(...new Uint8Array(passwordSaltHash)));

  // authentication = base64(sha256(base64_secret + challenge))
  const secretChallengeHash = await crypto.subtle.digest(
    "SHA-256",
    encoder.encode(base64Secret + challenge),
  );
  return btoa(String.fromCodePoint(...new Uint8Array(secretChallengeHash)));
}
