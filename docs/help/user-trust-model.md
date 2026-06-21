# User Trust Model

Koushi separates user trust, device state, and the final send decision. This
keeps the normal Matrix case from looking more dangerous than it is.

## User trust

### Unverified

This is the normal state for people you have not checked through another
channel. Messages can still be encrypted and sent. If a conversation needs
stronger assurance, verify the user with QR, emoji, or SAS.

### Verified

The user's identity has been checked through another channel. Use this for
people or rooms where impersonation resistance matters more than convenience.

### Identity reset

The user was verified before, but their current identity is different. This does
not prove compromise, but it means the previous verification can no longer be
used. Verify again, or explicitly forget the previous verification and treat the
user as unverified.

## Device state

Device state describes whether a device is cross-signed by its owner. It is not
the same as whether you have verified that user.

- **Cross-signed:** the owner identity signs this device.
- **Not cross-signed:** the device key exists, but the owner identity has not
  signed it.
- **Blocked:** the device is explicitly rejected.

## Effective trust

Effective trust is Koushi's send decision after combining user trust, device
state, blocked devices, and identity-reset warnings. A user can be unverified
while their devices are cross-signed by that user's own identity; that is still
different from a user you have verified yourself.

