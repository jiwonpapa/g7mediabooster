# ADR 0007: G7 7.1 core-free 어댑터

- 날짜: 2026-10-12
- 상태: 채택, 구현 완료 / 배포 인수 검증 전

## 결정

G7 코어·번들 게시판·기본 템플릿은 수정하지 않습니다.
실제 공개된 7.1.0 / `sirsoft-board` 1.2.0을 기준으로 layout response filter와 공식 선언형
middleware만 사용합니다. 기존 외부 첨부 patch capability 의존성을 제거합니다.

원본 저장 서비스를 감싸는 transaction에서 소유권·Ready·개수·삭제 경합을 확인하고 첨부 ID를
연결합니다. 기존 URL의 요청 terminal만 바꾸며 모든 native/third-party middleware를 유지합니다.
AttachmentService의 원본 권한 정책은 I/O 차단 storage로 재사용합니다.

## 이유와 제한

- 원본 갱신 때 source patch 충돌이 없고 원본 source 해시로 zero modification을 검증할 수 있습니다.
- 비밀·블라인드·삭제 정책을 복제하지 않아 원본과의 정책 분기를 줄입니다.
- 버전 문자열 외에 실제 공개 메서드 계약도 검사합니다. 미래 호환성을 보장하는 것은 아닙니다.
- 처리용 queue·스토리지·Rust 서버는 그대로 유지합니다. 새로운 서비스 컨테이너/프레임워크를 만들지 않습니다.
- 적용 해제는 신규 업로드 설정만 끕니다. 원격 미디어가 남아 있으면 전달 bridge는 제거하지 않습니다.
- 게시판 영상 poster 표시, 커스텀 템플릿, 실 브라우저·provider·ZIP 설치 종단은 별도 인수 게이트입니다.

과거 patched-host 증거는 역사 자료로만 유지하고 현재 배포 판정에 사용하지 않습니다.
