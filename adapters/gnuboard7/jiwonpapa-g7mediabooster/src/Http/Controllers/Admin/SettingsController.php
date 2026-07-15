<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Controllers\Admin;

use App\Http\Controllers\Api\Base\AdminBaseController;
use App\Services\ModuleSettingsService;
use Illuminate\Http\Client\Factory;
use Illuminate\Http\JsonResponse;
use InvalidArgumentException;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Http\Requests\UpdateSettingsRequest;
use Modules\Jiwonpapa\G7mediabooster\Services\HmacRequestSigner;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;
use Modules\Jiwonpapa\G7mediabooster\Services\WatermarkAssetCatalog;

final class SettingsController extends AdminBaseController
{
    private const MODULE = 'jiwonpapa-g7mediabooster';

    public function __construct(
        private readonly ModuleSettingsService $settings,
        private readonly HmacRequestSigner $signer,
        private readonly Factory $http,
        private readonly WatermarkAssetCatalog $watermarkAssets,
    ) {
        parent::__construct();
    }

    public function show(): JsonResponse
    {
        return $this->success('미디어 부스터 설정을 조회했습니다.', $this->safeSettings($this->allSettings()));
    }

    public function capabilities(): JsonResponse
    {
        try {
            $configuration = MediaBoosterConfiguration::fromArray($this->allSettings());
        } catch (InvalidArgumentException) {
            return $this->error('미디어 부스터 설정이 올바르지 않습니다.', 422);
        }
        if (! $configuration->enabled) {
            return $this->error('미디어 부스터가 비활성화되어 있습니다.', 503);
        }

        try {
            $capabilities = (new MediaBoosterClient($configuration, $this->signer, $this->http))->capabilities();
        } catch (MediaBoosterUpstreamException $error) {
            return $this->error(
                '미디어 처리 서버의 런타임 기능을 확인하지 못했습니다.',
                $error->httpStatus,
                ['capabilities' => [$error->errorCode]],
            );
        }
        $capabilities = $this->validatedCapabilities($capabilities);
        if ($capabilities === null) {
            return $this->error('미디어 처리 서버의 기능 응답이 올바르지 않습니다.', 502);
        }

        return $this->success('미디어 처리 서버의 런타임 기능을 확인했습니다.', $capabilities);
    }

    public function watermarkAssets(): JsonResponse
    {
        $adminId = (int) $this->getCurrentAdmin()?->getKey();
        if ($adminId < 1) {
            return $this->unauthorized('관리자 인증이 필요합니다.');
        }
        $selected = $this->allSettings()['watermark_asset_upload_id'] ?? '';
        if (! is_string($selected) || preg_match(
            '/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/',
            $selected,
        ) !== 1) {
            $selected = '';
        }

        return $this->success('워터마크 자산을 조회했습니다.', [
            'assets' => $this->watermarkAssets->forUser($adminId),
            'selected_upload_id' => strtolower($selected),
        ])->header('Cache-Control', 'private, no-store');
    }

    public function update(UpdateSettingsRequest $request): JsonResponse
    {
        $current = $this->allSettings();
        $incoming = $request->validated();
        if (($incoming['hmac_secret'] ?? '') === '') {
            $incoming['hmac_secret'] = (string) ($current['hmac_secret'] ?? '');
        }
        $candidate = array_replace($current, $incoming);
        $watermarkAssetId = strtolower(trim((string) ($candidate['watermark_asset_upload_id'] ?? '')));
        $adminId = (int) $this->getCurrentAdmin()?->getKey();
        if ($watermarkAssetId !== ''
            && ! $this->watermarkAssets->isSelectableForUser($adminId, $watermarkAssetId)
        ) {
            return $this->validationError(
                ['watermark_asset_upload_id' => ['선택할 수 없는 워터마크 자산입니다.']],
                '워터마크 자산을 다시 선택해 주세요.',
            );
        }
        $candidate['watermark_asset_upload_id'] = $watermarkAssetId;

        try {
            $configuration = MediaBoosterConfiguration::fromArray($candidate);
        } catch (InvalidArgumentException $error) {
            return $this->validationError(
                ['settings' => [$error->getMessage()]],
                '미디어 부스터 설정이 올바르지 않습니다.',
            );
        }

        if (! $configuration->enabled) {
            $candidate['policy_sync_state'] = 'disabled';
            $candidate['policy_sync_error'] = '';
            if (! $this->settings->save(self::MODULE, $candidate)) {
                return $this->error('미디어 부스터 설정을 저장하지 못했습니다.', 500);
            }

            return $this->success('미디어 부스터 설정을 저장했습니다.', $this->safeSettings($candidate));
        }

        $client = new MediaBoosterClient($configuration, $this->signer, $this->http);
        try {
            $active = $client->activeSitePolicy();
        } catch (MediaBoosterUpstreamException $error) {
            return $this->policySyncError($error, $candidate, false);
        }
        $activeRevision = $active['revision'] ?? 0;
        if (! is_int($activeRevision) || $activeRevision < 0 || $activeRevision >= PHP_INT_MAX) {
            return $this->error('미디어 처리 서버의 정책 revision 응답이 올바르지 않습니다.', 502);
        }
        $revision = $activeRevision + 1;
        $issuedAt = time();
        $candidate['policy_revision'] = $revision;
        $candidate['policy_sync_state'] = 'pending';
        $candidate['policy_sync_error'] = '';
        if (! $this->settings->save(self::MODULE, $candidate)) {
            return $this->error('미디어 부스터 설정을 저장하지 못했습니다.', 500);
        }

        try {
            $published = $client->publishSitePolicy([
                'schema_version' => 1,
                'revision' => $revision,
                'issued_at' => $issuedAt,
                'watermark' => $configuration->watermarkEnabled ? [
                    'asset_upload_id' => $configuration->watermarkAssetUploadId,
                    'position' => $configuration->watermarkPosition,
                    'margin_px' => $configuration->watermarkMarginPx,
                    'max_width_percent' => $configuration->watermarkMaxWidthPercent,
                    'opacity_percent' => $configuration->watermarkOpacityPercent,
                ] : null,
            ]);
        } catch (MediaBoosterUpstreamException $error) {
            return $this->policySyncError($error, $candidate, true);
        }
        $settingsHash = $published['settings_sha256'] ?? null;
        if (($published['revision'] ?? null) !== $revision
            || ! is_string($settingsHash)
            || preg_match('/^[a-f0-9]{64}$/', $settingsHash) !== 1) {
            $candidate['policy_sync_state'] = 'error';
            $candidate['policy_sync_error'] = 'INVALID_UPSTREAM_POLICY_RESPONSE';
            $this->settings->save(self::MODULE, $candidate);

            return $this->error('미디어 처리 서버의 정책 응답이 올바르지 않습니다.', 502);
        }
        $candidate['policy_settings_sha256'] = $settingsHash;
        $candidate['policy_sync_state'] = 'applied';
        $candidate['policy_sync_error'] = '';
        if (! $this->settings->save(self::MODULE, $candidate)) {
            return $this->error('정책은 적용됐지만 로컬 상태 저장에 실패했습니다.', 500);
        }

        return $this->success('미디어 부스터 설정과 정책을 적용했습니다.', $this->safeSettings($candidate));
    }

    /** @return array<string, mixed> */
    private function allSettings(): array
    {
        $settings = $this->settings->get(self::MODULE);

        return is_array($settings) ? $settings : [];
    }

    /**
     * @param array<string, mixed> $settings
     * @return array<string, mixed>
     */
    private function safeSettings(array $settings): array
    {
        $configured = is_string($settings['hmac_secret'] ?? null) && $settings['hmac_secret'] !== '';
        unset($settings['hmac_secret']);
        $settings['hmac_secret'] = '';
        $settings['hmac_secret_configured'] = $configured;

        return $settings;
    }

    /**
     * @param array<string, mixed> $capabilities
     * @return array<string, mixed>|null
     */
    private function validatedCapabilities(array $capabilities): ?array
    {
        $inputs = $this->validatedStringList($capabilities['image_inputs'] ?? null, 16, 32);
        $outputs = $this->validatedStringList($capabilities['image_outputs'] ?? null, 16, 32);
        $versions = $capabilities['native_versions'] ?? null;
        if ($inputs === null
            || $outputs === null
            || ! is_bool($capabilities['mp4_thumbnail'] ?? null)
            || ! is_bool($capabilities['mp4_h264_fallback'] ?? null)
            || ! is_array($versions)
            || array_is_list($versions)
            || count($versions) > 16) {
            return null;
        }
        foreach ($versions as $tool => $version) {
            if (! is_string($tool)
                || preg_match('/^[a-z0-9_-]{1,32}$/', $tool) !== 1
                || ! is_string($version)
                || $version === ''
                || strlen($version) > 256) {
                return null;
            }
        }

        return [
            'image_inputs' => $inputs,
            'image_outputs' => $outputs,
            'mp4_thumbnail' => $capabilities['mp4_thumbnail'],
            'mp4_h264_fallback' => $capabilities['mp4_h264_fallback'],
            'native_versions' => $versions,
        ];
    }

    /** @return list<string>|null */
    private function validatedStringList(mixed $value, int $maxItems, int $maxLength): ?array
    {
        if (! is_array($value) || ! array_is_list($value) || count($value) > $maxItems) {
            return null;
        }
        foreach ($value as $item) {
            if (! is_string($item)
                || preg_match('/^[a-z0-9_-]+$/', $item) !== 1
                || strlen($item) > $maxLength) {
                return null;
            }
        }

        return $value;
    }

    /**
     * @param array<string, mixed> $candidate
     */
    private function policySyncError(
        MediaBoosterUpstreamException $error,
        array $candidate,
        bool $pendingWasSaved,
    ): JsonResponse {
        if ($pendingWasSaved) {
            $candidate['policy_sync_state'] = 'error';
            $candidate['policy_sync_error'] = $error->errorCode;
            $this->settings->save(self::MODULE, $candidate);
        }

        return $this->error(
            '미디어 부스터 정책을 적용하지 못했습니다.',
            $error->httpStatus,
            ['policy' => [$error->errorCode]],
        );
    }
}
