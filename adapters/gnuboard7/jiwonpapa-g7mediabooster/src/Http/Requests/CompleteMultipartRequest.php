<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;
use Illuminate\Validation\Validator;

final class CompleteMultipartRequest extends FormRequest
{
    public function authorize(): bool
    {
        return true;
    }

    /** @return array<string, mixed> */
    public function rules(): array
    {
        return [
            'parts' => ['required', 'array', 'min:1', 'max:10000'],
            'parts.*.part_number' => ['required', 'integer', 'min:1', 'max:10000', 'distinct:strict'],
            'parts.*.etag' => ['required', 'string', 'max:1024', 'regex:/^[\x21-\x7e]+$/'],
        ];
    }

    /** @return array<int, callable> */
    public function after(): array
    {
        return [function (Validator $validator): void {
            foreach ((array) $this->input('parts', []) as $index => $part) {
                if (! is_array($part) || ($part['part_number'] ?? null) !== $index + 1) {
                    $validator->errors()->add('parts', 'part_number는 1부터 연속 오름차순이어야 합니다.');

                    return;
                }
            }
        }];
    }
}
