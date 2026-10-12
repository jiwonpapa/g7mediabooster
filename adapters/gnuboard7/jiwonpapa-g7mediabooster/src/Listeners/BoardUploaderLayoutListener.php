<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Listeners;

use App\Contracts\Extension\HookListenerInterface;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Services\BoardUploaderLayout;

final class BoardUploaderLayoutListener implements HookListenerInterface
{
    public function __construct(private readonly BoardUploaderLayout $layout, private readonly MediaBoosterConfiguration $config) {}

    public static function getSubscribedHooks(): array
    {
        return ['core.layout_extension.after_apply' => [
            'method' => 'apply', 'priority' => 120, 'type' => 'filter', 'sync' => true,
        ]];
    }

    public function handle(...$args): void {}

    public function apply(array $layout, int $templateId): array
    {
        $name = (string) ($layout['layout_name'] ?? '');

        return $this->config->enabled && (str_contains($name, 'board/form') || str_contains($name, 'admin_board_post_form'))
            ? $this->layout->apply($layout) : $layout;
    }
}
