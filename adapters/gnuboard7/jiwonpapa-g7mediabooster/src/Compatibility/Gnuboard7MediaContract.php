<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Compatibility;

use LogicException;
use ReflectionClass;
use ReflectionMethod;

/** Stock G7 7.1 capabilities used by the adapter; no upstream patch is required. */
final class Gnuboard7MediaContract
{
    public const CONTRACT_ID = 'g7mb.gnuboard7-core-free';

    public const CONTRACT_VERSION = '1.0.0';

    public static function assertCompatible(?string $hostRoot = null): void
    {
        $boardRoot = dirname((string) (new ReflectionClass('Modules\\Sirsoft\\Board\\Services\\AttachmentService'))->getFileName(), 3);
        $root = $hostRoot ?? (function_exists('app') && method_exists(app(), 'basePath') ? base_path() : $boardRoot);
        for ($depth = 0; $hostRoot === null && ! is_file($root.'/artisan') && $depth < 6; $depth++) {
            $root = dirname($root);
        }
        $manifestPath = $root.'/modules/sirsoft-board/module.json';
        if (! is_file($manifestPath)) {
            $manifestPath = $root.'/modules/_bundled/sirsoft-board/module.json';
        }
        $manifest = json_decode((string) file_get_contents($manifestPath), true, flags: JSON_THROW_ON_ERROR);
        if (! is_array($manifest) || version_compare((string) ($manifest['version'] ?? '0'), '1.2.0', '<')) {
            throw new LogicException('G7MB_G7_HOST_UNSUPPORTED');
        }
        self::assertRuntimeSurface();
    }

    /** Check the native metadata and normal authorization call surface. */
    public static function assertRuntimeSurface(): void
    {
        $service = 'Modules\\Sirsoft\\Board\\Services\\AttachmentService';
        $repository = 'Modules\\Sirsoft\\Board\\Repositories\\Contracts\\AttachmentRepositoryInterface';
        foreach ([
            [$service, 'getByHash', 2], [$service, 'download', 3], [$service, 'getFileInfo', 3],
            [$repository, 'create', 2], [$repository, 'linkAttachmentsByIds', 3],
            ['App\\Services\\LayoutExtensionService', 'applyExtensions', 3],
            ['App\\Extension\\AbstractModule', 'getMiddleware', 0],
        ] as [$class, $method, $count]) {
            if (! method_exists($class, $method)) {
                throw new LogicException('G7MB_G7_CAPABILITY_MISSING:'.$method);
            }
            $reflection = new ReflectionMethod($class, $method);
            if (! $reflection->isPublic() || $reflection->getNumberOfParameters() !== $count) {
                throw new LogicException('G7MB_G7_CAPABILITY_INCOMPATIBLE:'.$method);
            }
        }
    }
}
