<?php

declare(strict_types=1);

use Jiwonpapa\G7MediaBooster\Gnuboard5\SessionStore;

$host = getenv('G7MB_G5_TEST_MYSQL_HOST') ?: '127.0.0.1';
$port = (int) (getenv('G7MB_G5_TEST_MYSQL_PORT') ?: '3306');
$database = getenv('G7MB_G5_TEST_MYSQL_DATABASE') ?: 'g7mb_test';
$user = getenv('G7MB_G5_TEST_MYSQL_USER') ?: 'root';
$password = getenv('G7MB_G5_TEST_MYSQL_PASSWORD') ?: '';

mysqli_report(MYSQLI_REPORT_OFF);
$GLOBALS['g7mb_test_database'] = new mysqli($host, $user, $password, $database, $port);
if ($GLOBALS['g7mb_test_database']->connect_errno !== 0) {
    fwrite(STDERR, "MySQL connection failed\n");
    exit(1);
}
$GLOBALS['g7mb_test_database']->set_charset('utf8mb4');

/** @return mysqli_result|bool */
function sql_query(string $sql, bool $error = true): mysqli_result|bool
{
    return $GLOBALS['g7mb_test_database']->query($sql);
}

/** @return array<string, mixed>|false */
function sql_fetch(string $sql, bool $error = true): array|false
{
    $result = sql_query($sql, $error);
    if (! $result instanceof mysqli_result) {
        return false;
    }
    $row = $result->fetch_assoc();
    $result->free();

    return $row ?? [];
}

/** @return array<string, mixed>|false */
function sql_fetch_array(mysqli_result $result): array|false
{
    return $result->fetch_assoc() ?: false;
}

function sql_real_escape_string(string $value): string
{
    return $GLOBALS['g7mb_test_database']->real_escape_string($value);
}

function gate(bool $condition, string $label): void
{
    if (! $condition) {
        fwrite(STDERR, "FAIL: {$label}\n");
        exit(1);
    }
    fwrite(STDOUT, "PASS: {$label}\n");
}

$root = dirname(__DIR__);
require $root.'/adapters/gnuboard5/jiwonpapa-g7mediabooster/vendor/autoload.php';

sql_query('DROP TABLE IF EXISTS `g5_g7mb_upload_session`, `g5_board_file`, `g5_write_free`');
gate(sql_query("CREATE TABLE `g5_board_file` (
    `bo_table` varchar(20) NOT NULL,
    `wr_id` bigint unsigned NOT NULL,
    `bf_no` smallint unsigned NOT NULL,
    `bf_source` varchar(255) NOT NULL,
    `bf_file` varchar(255) NOT NULL,
    `bf_content` text NOT NULL,
    `bf_fileurl` text NOT NULL,
    `bf_thumburl` text NOT NULL,
    `bf_storage` varchar(32) NOT NULL,
    `bf_download` int NOT NULL,
    `bf_filesize` bigint unsigned NOT NULL,
    `bf_width` int NOT NULL,
    `bf_height` int NOT NULL,
    `bf_type` tinyint NOT NULL,
    `bf_datetime` datetime NOT NULL,
    KEY `g5_board_file_post` (`bo_table`, `wr_id`)
) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4") === true, 'G5 MyISAM board-file fixture');
gate(sql_query("CREATE TABLE `g5_write_free` (
    `wr_id` bigint unsigned NOT NULL,
    `wr_file` smallint unsigned NOT NULL DEFAULT 0,
    PRIMARY KEY (`wr_id`)
) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4") === true, 'G5 MyISAM write fixture');
sql_query('INSERT INTO `g5_write_free` (`wr_id`, `wr_file`) VALUES (1, 0), (2, 0)');

$store = new SessionStore('g5_g7mb_upload_session', 'g5_board_file');
$store->install();
$engine = sql_fetch("SELECT `ENGINE` FROM information_schema.TABLES
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'g5_g7mb_upload_session'");
gate(strtoupper((string) ($engine['ENGINE'] ?? '')) === 'INNODB', 'durable InnoDB upload session table');

$imageId = '018f47a0-1111-7111-8111-111111111111';
$videoId = '018f47a0-2222-7222-8222-222222222222';
$thirdId = '018f47a0-3333-7333-8333-333333333333';
$files = [
    [
        'client_ref' => 'image-1',
        'original_filename' => 'phone-photo.heic',
        'declared_kind' => 'image',
        'content_length' => 4_000_000,
        'content_type_hint' => 'image/heic',
    ],
    [
        'client_ref' => 'video-1',
        'original_filename' => 'clip.mp4',
        'declared_kind' => 'video',
        'content_length' => 20_000_000,
        'content_type_hint' => 'video/mp4',
    ],
];
$batch = [
    'batch_id' => '018f47a0-aaaa-7aaa-8aaa-aaaaaaaaaaaa',
    'uploads' => [
        ['upload_id' => $imageId, 'method' => 'single'],
        ['upload_id' => $videoId, 'method' => 'multipart'],
    ],
];
$store->recordBatch('m:tester', 'free', $files, $batch);
$store->markReady($imageId, [
    'mime_type' => 'image/jpeg',
    'size' => 3_000_000,
    'thumbnail_size' => 120_000,
    'preset_id' => 'default-v1',
]);
$store->markReady($videoId, [
    'mime_type' => 'video/mp4',
    'size' => 20_000_000,
    'thumbnail_size' => 140_000,
    'preset_id' => 'default-v1',
]);
$ready = $store->readyOwnedForLink([$imageId, $videoId], 'm:tester', 'free');
gate(count($ready) === 2, 'owned Ready session reservation');

$linked = $store->linkReady(
    $ready,
    'free',
    1,
    'g5_write_free',
    2,
    'http://127.0.0.1/plugin/g7mediabooster/delivery.php',
);
$post = sql_fetch('SELECT `wr_file` FROM `g5_write_free` WHERE `wr_id` = 1');
gate(
    $linked === [
        ['upload_id' => $imageId, 'bf_no' => 0],
        ['upload_id' => $videoId, 'bf_no' => 1],
    ] && (int) ($post['wr_file'] ?? -1) === 2,
    'atomic attachment numbering and wr_file recount',
);

sql_query("UPDATE `g5_g7mb_upload_session` SET `wr_id` = NULL, `bf_no` = NULL WHERE `upload_id` = '".
    sql_real_escape_string($imageId)."'");
$recovery = $store->readyOwnedForLink([$imageId], 'm:tester', 'free');
$relinked = $store->linkReady(
    $recovery,
    'free',
    1,
    'g5_write_free',
    2,
    'http://127.0.0.1/plugin/g7mediabooster/delivery.php',
);
$remoteCount = sql_fetch("SELECT COUNT(*) AS `cnt` FROM `g5_board_file` WHERE `bf_file` = 'g7mb-".
    sql_real_escape_string($imageId).".jpg'");
gate(
    $relinked === [['upload_id' => $imageId, 'bf_no' => 0]] && (int) ($remoteCount['cnt'] ?? 0) === 1,
    'MyISAM partial-link idempotent recovery',
);

$crossPostRejected = false;
try {
    $store->linkReady(
        $recovery,
        'free',
        2,
        'g5_write_free',
        2,
        'http://127.0.0.1/plugin/g7mediabooster/delivery.php',
    );
} catch (UnexpectedValueException) {
    $crossPostRejected = true;
}
gate($crossPostRejected, 'cross-post upload replay rejection');

$store->recordBatch('m:tester', 'free', [[
    'client_ref' => 'image-3',
    'original_filename' => 'extra.avif',
    'declared_kind' => 'image',
    'content_length' => 1_000_000,
    'content_type_hint' => 'image/avif',
]], [
    'batch_id' => '018f47a0-bbbb-7bbb-8bbb-bbbbbbbbbbbb',
    'uploads' => [['upload_id' => $thirdId, 'method' => 'single']],
]);
$store->markReady($thirdId, [
    'mime_type' => 'image/jpeg',
    'size' => 900_000,
    'thumbnail_size' => 80_000,
    'preset_id' => 'default-v1',
]);
$limitRejected = false;
try {
    $store->linkReady(
        $store->readyOwnedForLink([$thirdId], 'm:tester', 'free'),
        'free',
        1,
        'g5_write_free',
        2,
        'http://127.0.0.1/plugin/g7mediabooster/delivery.php',
    );
} catch (UnexpectedValueException) {
    $limitRejected = true;
}
gate($limitRejected, 'board attachment hard-limit rejection');

$file = sql_fetch("SELECT * FROM `g5_board_file` WHERE `bf_file` = 'g7mb-".
    sql_real_escape_string($imageId).".jpg'");
$store->scheduleDeletionForFile(is_array($file) ? $file : [], 0);
gate(count($store->dueDeletions(10)) === 1, 'durable deletion due queue');
$store->failDeletionRequest($imageId, 'UPSTREAM_UNAVAILABLE');
$retry = $store->find($imageId);
gate(
    (int) ($retry['deletion_attempts'] ?? 0) === 1
        && ($retry['deletion_last_error'] ?? null) === 'UPSTREAM_UNAVAILABLE'
        && strtotime((string) ($retry['deletion_due_at'] ?? '')) > time(),
    'bounded exponential deletion retry',
);
sql_query("UPDATE `g5_g7mb_upload_session` SET `deletion_due_at` = UTC_TIMESTAMP() WHERE `upload_id` = '".
    sql_real_escape_string($imageId)."'");
$store->completeDeletionRequest($imageId);
$deleted = $store->find($imageId);
gate(
    ($deleted['state'] ?? null) === 'deletion_pending'
        && ($deleted['deletion_requested_at'] ?? null) !== null
        && count($store->dueDeletions(10)) === 0,
    'idempotent deletion handoff completion',
);

fwrite(STDOUT, "Gnuboard5 SessionStore MySQL smoke: PASS (11/11)\n");
