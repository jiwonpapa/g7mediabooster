<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use Throwable;

final class DeliveryEndpoint
{
    public function __construct(private readonly GnuboardRuntime $runtime) {}

    public function run(): never
    {
        try {
            $boardTable = is_string($_GET['bo_table'] ?? null) ? $_GET['bo_table'] : '';
            $writeId = filter_var($_GET['wr_id'] ?? null, FILTER_VALIDATE_INT);
            $number = filter_var($_GET['no'] ?? null, FILTER_VALIDATE_INT);
            $variant = is_string($_GET['variant'] ?? null) ? $_GET['variant'] : '';
            if (! preg_match('/^[A-Za-z0-9_]{1,20}$/', $boardTable)
                || ! is_int($writeId)
                || $writeId < 1
                || ! is_int($number)
                || $number < 0
                || ! in_array($variant, ['master', 'thumbnail'], true)
            ) {
                throw new HttpFailure(404, 'ATTACHMENT_NOT_FOUND', '첨부파일을 찾을 수 없습니다.');
            }
            $fetchSite = strtolower((string) ($_SERVER['HTTP_SEC_FETCH_SITE'] ?? ''));
            if ($fetchSite !== '' && ! in_array($fetchSite, ['same-origin', 'same-site'], true)) {
                throw new HttpFailure(403, 'CROSS_SITE_REQUEST', '교차 사이트 첨부 요청은 허용되지 않습니다.');
            }
            if (! get_session('ss_view_'.$boardTable.'_'.$writeId)) {
                throw new HttpFailure(403, 'ATTACHMENT_FORBIDDEN', '게시글 열람 권한이 필요합니다.');
            }
            global $g5;
            $file = sql_fetch("SELECT * FROM `{$g5['board_file_table']}` WHERE `bo_table` = '".
                sql_real_escape_string($boardTable)."' AND `wr_id` = {$writeId} AND `bf_no` = {$number} LIMIT 1", false);
            if (! is_array($file) || $file === []) {
                throw new HttpFailure(404, 'ATTACHMENT_NOT_FOUND', '첨부파일을 찾을 수 없습니다.');
            }

            (new RemoteDelivery($this->runtime))->redirect($file, $boardTable, $writeId, $variant);
        } catch (HttpFailure $error) {
            $this->fail($error->status);
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 delivery denied: '.get_class($error));
            $this->fail(404);
        }
    }

    private function fail(int $status): never
    {
        http_response_code($status);
        header('Content-Type: text/plain; charset=utf-8');
        header('Cache-Control: private, no-store');
        header('X-Content-Type-Options: nosniff');
        echo $status === 403 ? 'Forbidden' : 'Not Found';
        exit;
    }
}
