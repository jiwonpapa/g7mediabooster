# G7 첨부 권한·보존 호스트 게이트 증거

- 기준 G7: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- 적용 patch: `0001` → `0002` → `0003` → `0004` → `0005`
- 격리 환경: 현재 patched checkout, 임시 MySQL 8.4, PHP 8.5.3, bundled module 0.4.0
- 정리: 일회용 module copy, `.env.testing`, MySQL container 제거 완료

## 통과 결과

- patch 5개 순차 clean apply와 canonical patch 일치: PASS
- 정적 upstream 계약: 28/28 + PHP/JSON parser PASS
- 현재 checkout 첨부·권한 회귀: 58 tests/90 assertions PASS
- 권한 경로: 10 tests/10 assertions PASS
- module host 보안·watermark catalog·보존 경로: 7 tests/38 assertions PASS

권한 행렬은 공개글 허용, 비밀글 작성자·`read-secret` 허용, 제3자 차단, 블라인드글
제3자 차단, 삭제글 작성자 차단·manager 허용, 게시글 누락 fail-closed와 native preview가
스토리지 접근 전에 같은 가드를 재사용하는지 확인합니다.

보존 행렬은 post soft-delete 예약, restore 전·후 예약 취소, 원격 삭제 요청 시작 뒤 복원 차단,
만료 row lease 선점, native attachment 재검사, 삭제 완료 상태 전이와 restore 경합 시 원격 요청 전
취소를 실제 G7 DB에서 확인합니다.

## 공식 범위 경계

이 결과로 patch `0001`~`0005` 적용 배포의 서버 측 첨부 권한과 보존 lease 계약을 공식 범위에
포함합니다. 브라우저에서 사용자별 403을 직접 관측하는 행렬과 보존 command가 실 R2/Lightsail
객체 삭제까지 완료하는 종단 증거는 별도 운영 게이트이며 통과 전 공식 기능으로 게시하지 않습니다.

재현 명령:

```bash
scripts/g7-host-security-gate.sh /path/to/patched-gnuboard7
```
