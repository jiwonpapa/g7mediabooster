# 얼굴 중심 크롭 보안 계약

## 공식 상태

얼굴 중심 크롭은 아직 미구현이며 공식 지원 기능이 아닙니다. 사람 신원을 판별하는 얼굴 인식은
제품 범위에 포함하지 않습니다. 향후 기능은 crop용 얼굴 위치와 focal point만 계산합니다.

## 구현 전 필수 조건

1. HTTP 요청 경로에서 detector를 실행하지 않습니다.
2. 업로드 worker의 별도 선택 queue에서만 실행합니다.
3. credential이 없는 no-network sandbox에 축소 preview만 전달합니다.
4. detector model/cascade 파일은 배포 manifest의 SHA-256과 정확히 일치해야 합니다.
5. pixel, input byte, CPU, RSS, timeout, process 동시성 hard limit를 둡니다.
6. 검출 결과는 bounding box와 정규화 focal point만 저장합니다.
7. 얼굴 원본, embedding, 신원 label은 저장하지 않습니다.
8. 실패·timeout·미검출은 중앙 crop으로 결정적으로 fallback 합니다.
9. focal-point revision을 derivative object key에 포함해 결과를 overwrite하지 않습니다.
10. 실제 crop·encode는 기존 libvips sandbox 경로만 사용합니다.

위 조건과 adversarial image 회귀 테스트가 모두 통과하기 전에는 설정 화면, capability,
공식 기능 목록에 얼굴 중심 크롭을 노출하지 않습니다.
