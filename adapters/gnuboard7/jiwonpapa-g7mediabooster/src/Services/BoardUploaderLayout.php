<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use LogicException;

/** Bounded transformation of stock board form responses, including separately served partials. */
final class BoardUploaderLayout
{
    private int $visited = 0;

    public function apply(array $layout): array
    {
        $this->visited = 0;

        return $this->visit($layout, 0);
    }

    private function visit(array $node, int $depth): array
    {
        if (++$this->visited > 20_000 || $depth > 64) {
            throw new LogicException('G7MB_LAYOUT_LIMIT_EXCEEDED');
        }
        $endpoint = $node['props']['apiEndpoints']['upload'] ?? null;
        if (($node['name'] ?? null) === 'FileUploader' && is_string($endpoint)
            && preg_match('#^/api/modules/sirsoft-board/(?:boards/|admin/board/).*/attachments$#', $endpoint)) {
            $admin = str_contains($endpoint, '/admin/board/');
            $mount = $admin ? 'g7mb-admin-uploader-mount' : 'g7mb-user-uploader-mount';

            return [
                'type' => 'basic', 'name' => 'Div', 'id' => $mount,
                'if' => $node['if'] ?? true,
                'props' => ['id' => $mount, 'className' => 'mb-4'],
                'lifecycle' => ['onMount' => [[
                    'handler' => 'jiwonpapa-g7mediabooster.mountUploader',
                    'params' => ['mountId' => $mount, 'boardSlug' => '{{route.slug}}',
                        'disabled' => $node['props']['disabled'] ?? false],
                ]]],
            ];
        }
        if (($node['name'] ?? null) === 'Button'
            && str_contains(json_encode($node['actions'] ?? [], JSON_THROW_ON_ERROR), 'upload:board_attachments')
            && ! str_contains((string) ($node['props']['disabled'] ?? ''), 'g7mbRuntimeReady')) {
            $disabled = $node['props']['disabled'] ?? false;
            $expression = is_string($disabled) && str_starts_with($disabled, '{{')
                ? substr($disabled, 2, -2) : ($disabled === true ? 'true' : 'false');
            $node['props']['disabled'] = '{{('.$expression.') || !_local?.g7mbRuntimeReady || _local?.g7mbUploading}}';
        }
        foreach ($node as $key => $value) {
            if (is_array($value)) {
                $node[$key] = $this->visit($value, $depth + 1);
            }
        }

        if (($node['handler'] ?? null) === 'sequence' && is_array($node['actions'] ?? null)) {
            $actions = [];
            $uploadSeen = false;
            foreach ($node['actions'] as $index => $action) {
                if ($uploadSeen && ($action['handler'] ?? null) === 'apiCall'
                    && str_contains((string) ($action['target'] ?? ''), '/api/modules/sirsoft-board/')
                    && str_contains((string) ($action['target'] ?? ''), '/posts')) {
                    $condition = $action['if'] ?? true;
                    if (! str_contains((string) $condition, '_eventResult?.success')) {
                        $expression = is_string($condition) && str_starts_with($condition, '{{')
                            ? substr($condition, 2, -2) : ($condition === false ? 'false' : 'true');
                        $action['if'] = '{{('.$expression.') && _local._eventResult?.success === true}}';
                    }
                    $uploadSeen = false;
                }
                $actions[] = $action;
                if (($action['handler'] ?? null) === 'emitEvent'
                    && ($action['params']['event'] ?? null) === 'upload:board_attachments') {
                    $uploadSeen = true;
                    if (! isset($node['actions'][$index + 1]['params']['form.attachment_ids'])) {
                        $actions[] = ['handler' => 'setState', 'params' => [
                            'target' => 'local',
                            'form.attachment_ids' => '{{_local._eventResult?.data?.attachment_ids ?? _local.form?.attachment_ids ?? []}}',
                        ]];
                    }
                }
            }
            $node['actions'] = $actions;
        }

        return $node;
    }
}
