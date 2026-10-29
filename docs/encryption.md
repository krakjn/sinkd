# Encryption at rest

libreflect sends deltas of the files it can see. Encrypt each file before it hits the sync tree and those deltas stop working, because the ciphertext changes far from the edit.

Encrypt the server store instead, so peers still ship plain deltas and the disk on the hub is unreadable without the machine key.

- Linux: fscrypt on `{sync_root}`, or LUKS under the filesystem that holds it.
- The client copies are the user's own files. Full-disk encryption on that machine covers them.

End-to-end encryption of the synced bytes is not part of this design. It would replace reflect's deltas with opaque blobs.
