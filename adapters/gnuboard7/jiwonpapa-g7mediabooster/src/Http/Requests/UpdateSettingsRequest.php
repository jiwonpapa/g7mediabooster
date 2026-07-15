<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;

final class UpdateSettingsRequest extends FormRequest
{
    public function authorize(): bool
    {
        return true;
    }

    /** @return array<string, mixed> */
    public function rules(): array
    {
        return [
            'enabled' => ['required', 'boolean'],
            'control_endpoint' => ['required', 'string', 'max:2048'],
            'key_id' => ['required', 'string', 'regex:/^[A-Za-z0-9_-]{1,128}$/'],
            'hmac_secret' => ['nullable', 'string', 'min:32', 'max:256'],
            'timeout_seconds' => ['required', 'integer', 'min:1', 'max:60'],
            'connect_timeout_seconds' => ['required', 'integer', 'min:1', 'max:15'],
            'max_parallel_files' => ['required', 'integer', 'min:1', 'max:16'],
            'max_parallel_parts' => ['required', 'integer', 'min:1', 'max:8'],
            'max_part_retries' => ['required', 'integer', 'min:0', 'max:5'],
            'status_poll_interval_ms' => ['required', 'integer', 'min:1500', 'max:10000'],
            'attachment_retention_days' => ['required', 'integer', 'min:1', 'max:365'],
            'watermark_enabled' => ['required', 'boolean'],
            'watermark_asset_upload_id' => ['nullable', 'string', 'uuid'],
            'watermark_position' => ['required', 'string', 'in:center,top_left,top_right,bottom_left,bottom_right'],
            'watermark_margin_px' => ['required', 'integer', 'min:0', 'max:1024'],
            'watermark_max_width_percent' => ['required', 'integer', 'min:1', 'max:50'],
            'watermark_opacity_percent' => ['required', 'integer', 'min:1', 'max:100'],
        ];
    }
}
