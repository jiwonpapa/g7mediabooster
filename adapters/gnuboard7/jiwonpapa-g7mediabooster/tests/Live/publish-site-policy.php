#!/usr/bin/env php
<?php

declare(strict_types=1);

use Illuminate\Http\Client\Factory;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Services\HmacRequestSigner;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;

$root = dirname(__DIR__, 2);
$autoload = $root.'/vendor/autoload.php';
if (! is_file($autoload)) {
    fwrite(STDERR, "module Composer dependencies are not installed\n");
    exit(2);
}
require $autoload;

/** @return non-empty-string */
function requiredEnvironment(string $name): string
{
    $value = getenv($name);
    if (! is_string($value) || $value === '') {
        throw new RuntimeException("missing required environment variable: {$name}");
    }

    return $value;
}

try {
    $endpoint = requiredEnvironment('G7MB_POLICY_ENDPOINT');
    $secret = requiredEnvironment('G7MB_POLICY_HMAC_SECRET');
    $assetUploadId = getenv('G7MB_POLICY_ASSET_UPLOAD_ID');
    $assetUploadId = is_string($assetUploadId) ? trim($assetUploadId) : '';
    $revision = filter_var(requiredEnvironment('G7MB_POLICY_REVISION'), FILTER_VALIDATE_INT);
    if (! is_int($revision) || $revision < 1) {
        throw new RuntimeException('G7MB_POLICY_REVISION must be a positive integer');
    }

    $configuration = MediaBoosterConfiguration::fromArray([
        'enabled' => true,
        'control_endpoint' => $endpoint,
        'key_id' => 'g7-primary',
        'hmac_secret' => $secret,
        'timeout_seconds' => 15,
        'connect_timeout_seconds' => 3,
        'max_parallel_files' => 8,
        'max_parallel_parts' => 4,
        'max_part_retries' => 3,
        'status_poll_interval_ms' => 1500,
        'attachment_retention_days' => 30,
        'watermark_enabled' => $assetUploadId !== '',
        'watermark_asset_upload_id' => $assetUploadId,
        'watermark_position' => 'bottom_right',
        'watermark_margin_px' => 0,
        'watermark_max_width_percent' => 20,
        'watermark_opacity_percent' => 70,
    ]);
    $client = new MediaBoosterClient($configuration, new HmacRequestSigner, new Factory);
    $watermark = $assetUploadId === '' ? null : [
        'asset_upload_id' => $assetUploadId,
        'position' => 'bottom_right',
        'margin_px' => 0,
        'max_width_percent' => 20,
        'opacity_percent' => 70,
    ];
    $published = $client->publishSitePolicy([
        'schema_version' => 1,
        'revision' => $revision,
        'issued_at' => time(),
        'watermark' => $watermark,
    ]);
    $active = $client->activeSitePolicy();
    if (($published['revision'] ?? null) !== $revision
        || ($active['revision'] ?? null) !== $revision
        || ! is_string($published['settings_sha256'] ?? null)
        || preg_match('/^[a-f0-9]{64}$/', $published['settings_sha256']) !== 1
        || ($assetUploadId === '' && ($active['watermark'] ?? null) !== null)
        || ($assetUploadId !== '' && ($active['watermark']['asset_upload_id'] ?? null) !== $assetUploadId)
    ) {
        throw new RuntimeException('published site policy did not become the exact active revision');
    }

    echo json_encode([
        'revision' => $revision,
        'settings_sha256' => $published['settings_sha256'],
        'watermark' => $active['watermark'] ?? null,
    ], JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES)."\n";
} catch (Throwable $error) {
    fwrite(STDERR, 'G7 policy client smoke failed: '.$error->getMessage()."\n");
    exit(1);
}
