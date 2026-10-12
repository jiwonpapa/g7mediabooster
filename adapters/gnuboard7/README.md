# Gnuboard 7 adapter

실제 모듈은 [`jiwonpapa-g7mediabooster`](jiwonpapa-g7mediabooster)에 있습니다.
0.5.1 기준은 공식 G7 7.1.0 / 번들 `sirsoft-board` 1.2.0입니다.

- 코어·게시판·기본 템플릿 원본 수정 없이 공개 layout filter와 선언형 middleware로 연결
- 기존 저장 이벤트를 기다려 다중 직접 업로드 → Ready 검증 → native attachment 연결
- 기존 다운로드·미리보기 URL을 유지하고 원본 게시판 권한 검사 후 원격 파일 전달
- 업로드 비활성화 시 기존 업로더 복원, 이미 올린 원격 첨부 열람 유지
- PHP에는 파일 바이트나 저장소 비밀값을 전달하지 않음

기존 [`upstream-contract`](upstream-contract) patch는 역사적 제안이며 현재 설치·검증·배포에 사용하지 않습니다.
g7devops.com ZIP 업데이트·브라우저·R2 canary는 통과했습니다. 정식 배포는 남은 stock 첫 설치·전체 인수 게이트 전까지 차단합니다.
[실서버 검증](../../docs/evidence/G7DEVOPS_CORE_FREE_R2_E2E_20261012.md)을 참고하십시오.
[연동 설명](../../docs/GNUBOARD7_INTEGRATION.md)과 [검증 보고서](../../docs/evidence/G7_CORE_FREE_710_20261012.md)를 참고하십시오.
