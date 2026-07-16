<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Compatibility;

use JsonException;
use LogicException;
use ReflectionClass;
use ReflectionException;
use ReflectionMethod;

/**
 * Verifies the Gnuboard7 host contract required by the media attachment bridge.
 *
 * The module version dependency alone is insufficient because current G7 only
 * checks whether a dependency is active. This gate verifies a versioned host
 * capability, the callable PHP surface, and the layout targets before module
 * activation. A missing or malformed signal always fails closed.
 */
final class Gnuboard7MediaContract
{
    public const CONTRACT_ID = 'sirsoft-board.secure-external-attachments';

    public const CONTRACT_VERSION = '1.0.0';

    /** @var list<string> */
    private const REQUIRED_FEATURES = [
        'bounded_attachment_ids',
        'owner_scoped_linking',
        'all_or_nothing_linking',
        'attachment_count_sync',
        'byte_free_delivery_authorization',
        'visibility_aware_delivery',
        'filtered_download_url',
        'filtered_preview_url',
        'video_poster_url',
        'stable_user_layout_targets',
        'stable_admin_layout_targets',
        'module_prefixed_layout_overlays',
        'all_matching_layout_targets',
    ];

    /**
     * Assert that the running G7 host can safely activate the bridge.
     *
     * @param string|null $hostRoot Explicit G7 root for offline verification.
     */
    public static function assertCompatible(?string $hostRoot = null): void
    {
        $root = self::resolveHostRoot($hostRoot);
        $boardRoot = $root.'/modules/_bundled/sirsoft-board';

        self::assertBoardVersion($boardRoot.'/module.json');
        self::assertContractDocument(self::readJson($boardRoot.'/resources/contracts/external-media-v1.json'));
        self::assertRuntimeSurface();
        self::assertLayoutTargets($root, $boardRoot);
    }

    /**
     * Validate a decoded public capability document.
     *
     * Exposed for packaging and unit-test harnesses; callers should use
     * {@see assertCompatible()} for a complete host decision.
     *
     * @param array<string, mixed> $contract
     */
    public static function assertContractDocument(array $contract): void
    {
        if (($contract['schema_version'] ?? null) !== 1
            || ($contract['id'] ?? null) !== self::CONTRACT_ID
            || ! is_string($contract['version'] ?? null)
            || version_compare($contract['version'], self::CONTRACT_VERSION, '<')
            || version_compare($contract['version'], '2.0.0', '>=')) {
            throw new LogicException('G7MB_G7_CONTRACT_VERSION_UNSUPPORTED');
        }

        $features = $contract['features'] ?? null;
        if (! is_array($features) || count($features) > 64) {
            throw new LogicException('G7MB_G7_CONTRACT_FEATURES_INVALID');
        }
        foreach ($features as $feature) {
            if (! is_string($feature) || $feature === '' || strlen($feature) > 64) {
                throw new LogicException('G7MB_G7_CONTRACT_FEATURES_INVALID');
            }
        }
        if (array_values($features) !== array_values(array_unique($features))) {
            throw new LogicException('G7MB_G7_CONTRACT_FEATURES_INVALID');
        }

        foreach (self::REQUIRED_FEATURES as $feature) {
            if (! in_array($feature, $features, true)) {
                throw new LogicException('G7MB_G7_CONTRACT_FEATURE_MISSING:'.$feature);
            }
        }
    }

    private static function resolveHostRoot(?string $hostRoot): string
    {
        if ($hostRoot !== null) {
            $resolved = realpath($hostRoot);
            if ($resolved === false || ! is_dir($resolved.'/modules/_bundled/sirsoft-board')) {
                throw new LogicException('G7MB_G7_HOST_ROOT_INVALID');
            }

            return $resolved;
        }

        if (function_exists('base_path')) {
            $resolved = realpath((string) base_path());
            if ($resolved !== false) {
                return $resolved;
            }
        }

        try {
            $source = (new ReflectionClass('Modules\\Sirsoft\\Board\\Services\\AttachmentService'))->getFileName();
        } catch (ReflectionException) {
            $source = false;
        }
        if (! is_string($source)) {
            throw new LogicException('G7MB_G7_HOST_ROOT_UNRESOLVED');
        }

        $resolved = realpath(dirname($source, 6));
        if ($resolved === false) {
            throw new LogicException('G7MB_G7_HOST_ROOT_UNRESOLVED');
        }

        return $resolved;
    }

    private static function assertBoardVersion(string $manifestPath): void
    {
        $manifest = self::readJson($manifestPath);
        $version = $manifest['version'] ?? null;
        if (! is_string($version)
            || version_compare($version, '1.2.0', '<')
            || version_compare($version, '2.0.0', '>=')) {
            throw new LogicException('G7MB_SIRSOFT_BOARD_VERSION_UNSUPPORTED');
        }
    }

    private static function assertRuntimeSurface(): void
    {
        self::assertMethod(
            'Modules\\Sirsoft\\Board\\Services\\AttachmentService',
            'authorizeDelivery',
            ['slug', 'id', 'context'],
            2,
        );
        self::assertMethod(
            'Modules\\Sirsoft\\Board\\Repositories\\Contracts\\AttachmentRepositoryInterface',
            'findPostForAttachmentDelivery',
            ['slug', 'postId'],
            2,
        );
        self::assertMethod(
            'Modules\\Sirsoft\\Board\\Repositories\\Contracts\\AttachmentRepositoryInterface',
            'linkAttachmentsByIds',
            ['slug', 'ids', 'postId', 'ownerId'],
            4,
        );
    }

    private static function assertMethod(
        string $class,
        string $method,
        array $parameterNames,
        int $requiredParameters,
    ): void {
        if (! class_exists($class) && ! interface_exists($class)) {
            throw new LogicException('G7MB_G7_CONTRACT_CLASS_MISSING:'.$class);
        }

        try {
            $reflection = new ReflectionMethod($class, $method);
        } catch (ReflectionException) {
            throw new LogicException('G7MB_G7_CONTRACT_METHOD_MISSING:'.$class.'::'.$method);
        }
        $actualNames = array_map(
            static fn (\ReflectionParameter $parameter): string => $parameter->getName(),
            $reflection->getParameters(),
        );
        if (! $reflection->isPublic()
            || $reflection->isStatic()
            || $actualNames !== $parameterNames
            || $reflection->getNumberOfRequiredParameters() !== $requiredParameters) {
            throw new LogicException('G7MB_G7_CONTRACT_METHOD_INCOMPATIBLE:'.$class.'::'.$method);
        }
    }

    private static function assertLayoutTargets(string $root, string $boardRoot): void
    {
        self::assertJsonIds(
            $root.'/templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json',
            ['board_native_attachment_section', 'board_native_file_uploader', 'board_post_submit'],
        );
        self::assertJsonIds(
            $boardRoot.'/resources/layouts/admin/partials/admin_board_post_form/_attachments.json',
            ['admin_board_native_file_uploader'],
        );
        self::assertJsonIds(
            $boardRoot.'/resources/layouts/admin/admin_board_post_form.json',
            ['footer_save_button'],
        );
    }

    /** @param list<string> $requiredIds */
    private static function assertJsonIds(string $path, array $requiredIds): void
    {
        $ids = [];
        self::collectIds(self::readJson($path), $ids);
        foreach ($requiredIds as $id) {
            if (! in_array($id, $ids, true)) {
                throw new LogicException('G7MB_G7_LAYOUT_TARGET_MISSING:'.$id);
            }
        }
    }

    /**
     * @param array<mixed> $node
     * @param list<string> $ids
     */
    private static function collectIds(array $node, array &$ids): void
    {
        foreach ($node as $key => $value) {
            if ($key === 'id' && is_string($value)) {
                $ids[] = $value;
            }
            if (is_array($value)) {
                self::collectIds($value, $ids);
            }
        }
    }

    /** @return array<string, mixed> */
    private static function readJson(string $path): array
    {
        if (! is_file($path) || ! is_readable($path)) {
            throw new LogicException('G7MB_G7_CONTRACT_FILE_MISSING');
        }
        $bytes = filesize($path);
        if (! is_int($bytes) || $bytes < 2 || $bytes > 65_536) {
            throw new LogicException('G7MB_G7_CONTRACT_FILE_SIZE_INVALID');
        }

        try {
            $decoded = json_decode((string) file_get_contents($path), true, 64, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            throw new LogicException('G7MB_G7_CONTRACT_JSON_INVALID');
        }
        if (! is_array($decoded)) {
            throw new LogicException('G7MB_G7_CONTRACT_JSON_INVALID');
        }

        return $decoded;
    }
}
