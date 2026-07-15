<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster;

use App\Extension\AbstractModule;
use Modules\Jiwonpapa\G7mediabooster\Listeners\AttachmentUrlListener;

final class Module extends AbstractModule
{
    /** @return array<class-string> */
    public function getHookListeners(): array
    {
        return [AttachmentUrlListener::class];
    }

    /**
     * @return array<string, mixed>
     */
    public function getConfigValues(): array
    {
        $path = $this->getSettingsDefaultsPath();
        if ($path === null) {
            return [];
        }

        $decoded = json_decode((string) file_get_contents($path), true);

        return is_array($decoded['defaults'] ?? null) ? $decoded['defaults'] : [];
    }

    /**
     * Flat settings are deliberate: the current G7 settings service encrypts
     * sensitive fields at the top level.
     *
     * @return array<string, array<string, mixed>>
     */
    public function getSettingsSchema(): array
    {
        return [
            'enabled' => ['type' => 'boolean'],
            'control_endpoint' => ['type' => 'string'],
            'key_id' => ['type' => 'string'],
            'hmac_secret' => ['type' => 'string', 'sensitive' => true],
            'timeout_seconds' => ['type' => 'integer'],
            'connect_timeout_seconds' => ['type' => 'integer'],
            'max_parallel_files' => ['type' => 'integer'],
            'max_parallel_parts' => ['type' => 'integer'],
            'max_part_retries' => ['type' => 'integer'],
            'status_poll_interval_ms' => ['type' => 'integer'],
        ];
    }

    /**
     * @return array<string, mixed>
     */
    public function getPermissions(): array
    {
        return [
            'name' => ['ko' => 'G7 미디어 부스터', 'en' => 'G7 Media Booster'],
            'description' => [
                'ko' => '미디어 업로드와 환경설정 권한',
                'en' => 'Media upload and configuration permissions',
            ],
            'categories' => [
                [
                    'identifier' => 'settings',
                    'name' => ['ko' => '환경설정', 'en' => 'Settings'],
                    'description' => ['ko' => '미디어 부스터 설정', 'en' => 'Media Booster settings'],
                    'permissions' => [
                        [
                            'action' => 'read',
                            'name' => ['ko' => '설정 조회', 'en' => 'View settings'],
                            'description' => ['ko' => '설정과 연결 상태를 조회합니다.', 'en' => 'View settings and connection state.'],
                            'type' => 'admin',
                            'roles' => ['admin'],
                        ],
                        [
                            'action' => 'update',
                            'name' => ['ko' => '설정 변경', 'en' => 'Update settings'],
                            'description' => ['ko' => '제어 API와 업로더 설정을 변경합니다.', 'en' => 'Update control API and uploader settings.'],
                            'type' => 'admin',
                            'roles' => ['admin'],
                        ],
                    ],
                ],
            ],
        ];
    }

    /**
     * @return array<int, array<string, mixed>>
     */
    public function getAdminMenus(): array
    {
        return [[
            'name' => ['ko' => '미디어 부스터', 'en' => 'Media Booster'],
            'slug' => 'jiwonpapa-g7mediabooster',
            'url' => '/admin/media-booster/settings',
            'icon' => 'fas fa-photo-film',
            'order' => 45,
            'permission' => 'jiwonpapa-g7mediabooster.settings.read',
        ]];
    }
}
