# rave

Bevy + libp2p rave party. Peers walk, see each other, chat — over
`relaye.sbvh.nl`. Identity in IndexedDB.

Deployed at https://rave.sbvh.nl/ via CI on push to `rave` or `master`
(paths filter `rave/**`). No local dev — Bevy compile cost lives in
CI, not on this machine. See `.github/workflows/deploy-rave.yml`.

## Build

```
nix develop -c make wasm
```

Nix flakes only see git-tracked files. New files need `git add` before
`nix develop` sees them.
