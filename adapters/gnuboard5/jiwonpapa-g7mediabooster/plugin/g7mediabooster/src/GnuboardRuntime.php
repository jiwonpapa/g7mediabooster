<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use RuntimeException;

final class GnuboardRuntime
{
    private ?Configuration $configuration = null;

    private ?SessionStore $store = null;

    private ?ControlClient $client = null;

    public function configuration(): Configuration
    {
        return $this->configuration ??= Configuration::fromEnvironment();
    }

    public function store(): SessionStore
    {
        if ($this->store !== null) {
            return $this->store;
        }
        global $g5;
        $sessionTable = $g5['g7mb_upload_session_table'] ?? null;
        $boardFileTable = $g5['board_file_table'] ?? null;
        if (! is_string($sessionTable) || ! is_string($boardFileTable)) {
            throw new RuntimeException('Gnuboard table configuration is unavailable');
        }

        return $this->store = new SessionStore($sessionTable, $boardFileTable);
    }

    public function client(): ControlClient
    {
        return $this->client ??= new ControlClient(
            $this->configuration(),
            new HmacSigner,
            new CurlTransport,
        );
    }

    /** @return array<string, mixed> */
    public function board(string $boardTable): array
    {
        if (! preg_match('/^[A-Za-z0-9_]{1,20}$/', $boardTable)) {
            throw new HttpFailure(404, 'BOARD_NOT_FOUND', '게시판을 찾을 수 없습니다.');
        }
        $board = get_board_db($boardTable, false);
        if (! is_array($board) || ($board['bo_table'] ?? null) !== $boardTable) {
            throw new HttpFailure(404, 'BOARD_NOT_FOUND', '게시판을 찾을 수 없습니다.');
        }

        return $board;
    }

    /** @param array<string, mixed> $board */
    public function assertUploadPermission(array $board): void
    {
        global $member, $is_admin;
        $level = (int) ($member['mb_level'] ?? 1);
        $isManager = is_string($is_admin ?? null) && $is_admin !== '';
        $writeLevel = max(1, (int) ($board['bo_write_level'] ?? 1));
        $uploadLevel = max(1, (int) ($board['bo_upload_level'] ?? 1));
        if (! $isManager && ($level < $writeLevel || $level < $uploadLevel)) {
            throw new HttpFailure(403, 'UPLOAD_FORBIDDEN', '이 게시판에 파일을 업로드할 권한이 없습니다.');
        }
        if ((int) ($board['bo_upload_count'] ?? 0) < 1 || (int) ($board['bo_upload_size'] ?? 0) < 1) {
            throw new HttpFailure(403, 'UPLOAD_DISABLED', '이 게시판은 파일 업로드를 사용하지 않습니다.');
        }
    }

    public function ownerKey(): string
    {
        global $member;
        $memberId = is_string($member['mb_id'] ?? null) ? trim($member['mb_id']) : '';
        if ($memberId !== '') {
            if (! preg_match('/^[A-Za-z0-9_.@-]{1,20}$/', $memberId)) {
                throw new HttpFailure(403, 'INVALID_MEMBER_SCOPE', '사용자 업로드 범위를 확인할 수 없습니다.');
            }

            return 'm:'.$memberId;
        }
        $sessionId = session_id();
        if ($sessionId === '') {
            throw new HttpFailure(403, 'SESSION_REQUIRED', '업로드 세션을 확인할 수 없습니다.');
        }
        $configuration = $this->configuration();
        if (! $configuration->enabled) {
            throw new HttpFailure(503, 'ADAPTER_DISABLED', '미디어 부스터가 비활성화되어 있습니다.');
        }

        return 'g:'.hash_hmac('sha256', $sessionId, $configuration->hmacSecret);
    }

    public function csrfToken(): string
    {
        $token = get_session('ss_g7mb_csrf');
        if (! is_string($token) || ! preg_match('/^[a-f0-9]{64}$/', $token)) {
            $token = bin2hex(random_bytes(32));
            set_session('ss_g7mb_csrf', $token);
        }

        return $token;
    }

    public function assertCsrf(): void
    {
        $provided = $_SERVER['HTTP_X_G7MB_CSRF'] ?? null;
        $expected = get_session('ss_g7mb_csrf');
        if (! is_string($provided)
            || ! is_string($expected)
            || ! preg_match('/^[a-f0-9]{64}$/', $provided)
            || ! hash_equals($expected, $provided)
        ) {
            throw new HttpFailure(403, 'CSRF_FAILED', '업로드 요청 토큰이 올바르지 않습니다.');
        }
        $fetchSite = strtolower((string) ($_SERVER['HTTP_SEC_FETCH_SITE'] ?? ''));
        if ($fetchSite !== '' && ! in_array($fetchSite, ['same-origin', 'same-site'], true)) {
            throw new HttpFailure(403, 'CROSS_SITE_REQUEST', '교차 사이트 업로드 요청은 허용되지 않습니다.');
        }
    }

    public function apiUrl(): string
    {
        return G5_PLUGIN_URL.'/g7mediabooster/api.php';
    }

    public function deliveryUrl(): string
    {
        return G5_PLUGIN_URL.'/g7mediabooster/delivery.php';
    }
}
