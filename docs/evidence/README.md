# 검증 증거 인덱스

이 디렉터리의 날짜 문서는 실행 당시의 불변 snapshot입니다. 과거 PASS는 현재 제품 전체나
다른 provider·G7 버전의 PASS를 뜻하지 않습니다.

현재 공개 가능 여부와 차단 사유의 정본은
[`deploy/official-features-v1.json`](../../deploy/official-features-v1.json)입니다. CI는 증거 경로,
manifest 버전과 공개 상태를 함께 검사합니다.

- Cloudflare R2 현재 실계정 증거: `G7DEVOPS_LIVE_R2_E2E_20260717.md`
- G7 과거 patched-host 증거: `G7_UPSTREAM_CONTRACT_20260716.md` 등
- 공식 stock G7 7.0.6 현재 판정: CI capability fail-closed gate와 기능 상태 정본
