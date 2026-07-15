<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use UnexpectedValueException;

final class DeliveryValidator
{
    /** @param array<string, mixed> $delivery */
    public function validate(array $delivery, string $uploadId, string $variant): string
    {
        if (($delivery['upload_id'] ?? null) !== $uploadId
            || ($delivery['variant'] ?? null) !== $variant
            || ! in_array($delivery['content_type'] ?? null, ['image/jpeg', 'video/mp4', 'video/quicktime'], true)
            || ! is_int($delivery['byte_len'] ?? null)
            || $delivery['byte_len'] < 1
            || ! is_string($delivery['expires_at'] ?? null)
            || strtotime($delivery['expires_at']) === false
            || strtotime($delivery['expires_at']) <= time()
            || ! is_string($delivery['delivery_url'] ?? null)
        ) {
            throw new UnexpectedValueException('invalid derivative delivery response');
        }
        $url = $delivery['delivery_url'];
        if (strlen($url) > 8192 || preg_match('/[\r\n]/', $url)) {
            throw new UnexpectedValueException('invalid derivative delivery URL');
        }
        $parts = parse_url($url);
        $host = strtolower((string) ($parts['host'] ?? ''));
        $scheme = strtolower((string) ($parts['scheme'] ?? ''));
        if (($scheme !== 'https' && ! ($scheme === 'http' && in_array($host, ['127.0.0.1', '::1', 'localhost'], true)))
            || $host === ''
            || isset($parts['user'])
            || isset($parts['pass'])
            || isset($parts['fragment'])
        ) {
            throw new UnexpectedValueException('invalid derivative delivery URL');
        }

        return $url;
    }
}
