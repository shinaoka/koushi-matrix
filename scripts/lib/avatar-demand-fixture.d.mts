export const AVATAR_FIXTURE_READERS: 1500;
export function setFixtureAvatar(homeserver: string, registration: {
  access_token: string;
  user_id: string;
}): Promise<string>;

export function seedAvatarDemandFixture(options: {
  homeserver: string;
  ownerAccessToken: string;
  runId: string;
}): Promise<{
  roomId: string;
  eventId: string;
  readerCount: number;
  receiptReadback: { stored: number | null; packed: number | null };
}>;

export function classifyAvatarReceiptReadback(options: {
  serverKind: string;
  capability: { supported: boolean; limitation: string };
  readback: { stored: number | null; packed: number | null };
  expected: number;
}): { limited: false } | { limited: true; token: string };
