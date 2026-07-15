<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;
use Illuminate\Validation\Rule;
use Illuminate\Validation\Validator;

final class CreateUploadBatchRequest extends FormRequest
{
    private const IMAGE_MAX_BYTES = 128 * 1024 * 1024;
    private const VIDEO_MAX_BYTES = 5 * 1024 * 1024 * 1024;

    /** @var array<int, string> */
    private const CONTENT_TYPES = [
        'application/octet-stream',
        'image/avif',
        'image/gif',
        'image/heic',
        'image/heif',
        'image/jpeg',
        'image/png',
        'image/webp',
        'video/mp4',
    ];

    public function authorize(): bool
    {
        return true;
    }

    /**
     * @return array<string, mixed>
     */
    public function rules(): array
    {
        return [
            'files' => ['required', 'array', 'list', 'min:1', 'max:100'],
            'files.*.client_ref' => ['required', 'string', 'max:128', 'regex:/^[A-Za-z0-9_-]+$/', 'distinct:strict'],
            'files.*.original_filename' => [
                'required',
                'string',
                'max:255',
                'regex:#\A[^\x00-\x1F\x7F/\\\\]+\z#u',
            ],
            'files.*.declared_kind' => ['required', Rule::in(['image', 'video'])],
            'files.*.content_length' => ['required', 'integer', 'min:1', 'max:'.self::VIDEO_MAX_BYTES],
            'files.*.content_type_hint' => ['required', 'string', Rule::in(self::CONTENT_TYPES)],
        ];
    }

    /**
     * @return array<int, callable>
     */
    public function after(): array
    {
        return [function (Validator $validator): void {
            foreach ((array) $this->input('files', []) as $index => $file) {
                if (! is_array($file)) {
                    continue;
                }
                $kind = $file['declared_kind'] ?? null;
                $size = filter_var($file['content_length'] ?? null, FILTER_VALIDATE_INT);
                $type = $file['content_type_hint'] ?? null;
                if ($kind === 'image' && is_int($size) && $size > self::IMAGE_MAX_BYTES) {
                    $validator->errors()->add("files.{$index}.content_length", '이미지는 128 MiB를 초과할 수 없습니다.');
                }
                if (is_string($type) && $type !== 'application/octet-stream') {
                    if ($kind === 'image' && ! str_starts_with($type, 'image/')) {
                        $validator->errors()->add("files.{$index}.content_type_hint", '이미지 종류와 MIME 힌트가 일치하지 않습니다.');
                    }
                    if ($kind === 'video' && ! str_starts_with($type, 'video/')) {
                        $validator->errors()->add("files.{$index}.content_type_hint", '영상 종류와 MIME 힌트가 일치하지 않습니다.');
                    }
                }
            }
        }];
    }

    protected function prepareForValidation(): void
    {
        $files = $this->input('files');
        if (! is_array($files)) {
            return;
        }

        foreach ($files as &$file) {
            if (is_array($file) && is_string($file['content_type_hint'] ?? null)) {
                $file['content_type_hint'] = strtolower(trim($file['content_type_hint']));
            }
        }
        unset($file);
        $this->merge(['files' => $files]);
    }
}
