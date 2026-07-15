# Gnuboard 5 adapter

G5 세션·게시판 권한을 확인하고 G7MediaBooster HMAC 요청을 생성하는 얇은 PHP 어댑터가
들어갈 자리입니다. object key, MIME 승인, 변환 preset과 작업 상태는 Rust가 소유합니다.

구현 시 OpenAPI fixture와 G5 실제 업로드 화면 smoke를 같은 변경에 포함해야 합니다.
