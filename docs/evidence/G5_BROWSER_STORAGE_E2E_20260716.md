# G5 browser/storage E2E evidence

- Date: 2026-07-16 KST
- Gnuboard: 5.6.24, commit `d6912e0ca`
- Runtime: actual G5 PHP server, MySQL 8.4 with native MyISAM board tables, MinIO, Rust API/worker
- Method: isolated Playwright browser session; the Gnuboard source itself was not modified

## Result

| Gate | Result |
|---|---|
| G5 source contract | PASS 21/21 |
| PHP unit | PASS 14 tests, 25 assertions |
| TypeScript unit | PASS 5 tests |
| TypeScript typecheck / production IIFE | PASS |
| MySQL session/link/delete host smoke | PASS 11/11 |
| Browser small image single PUT | PASS |
| Browser 9,796,467-byte image 2-part multipart | PASS |
| Rust safety processing and Ready | PASS 2/2 |
| `g5_board_file` native remote rows | PASS 2/2, `wr_file=2` |
| Authenticated private thumbnails | PASS, decoded 8x8 and 1280x960 |
| Anonymous private thumbnail | PASS, HTTP 403 |

Both files were selected in one browser batch. File bytes went directly to object storage; PHP handled
only same-origin control calls and the final post attachment identifiers. The multipart browser path
received and exposed an ETag for each part, completed the upload, and displayed both processed JPEG
thumbnails on the saved G5 post.

## Compatibility defects caught and fixed

- actual G5 table prefix constant and `get_board_db()` API mismatch
- legacy hook dispatcher incompatibility with static callable arrays on PHP 8
- browser configuration global overwritten by the Vite IIFE name
- multipart response validators assuming fields not present in the Rust contract
- G5's global POST escaping corrupting a JSON hidden field; replaced with strict UUID CSV
- stale Ready upload identifiers remaining hidden after the visible file selection was replaced

## Support boundary

This evidence promotes only G5 5.6.24 with the packaged core-free adapter and the product's runtime S3
operation subset. Real R2/Lightsail credentials, 5 GiB transfer, other G5 versions, and provider-side
retention deletion remain separate release gates and are not claimed as verified here.
