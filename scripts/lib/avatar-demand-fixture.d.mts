export const AVATAR_FIXTURE_READERS: 1500;

export function seedAvatarDemandFixture(options: {
  homeserver: string;
  ownerAccessToken: string;
  runId: string;
}): Promise<{ roomId: string; eventId: string; readerCount: number }>;
