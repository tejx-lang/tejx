import http from 'k6/http';
import { check } from 'k6';

export const options = {
    stages: [
        { duration: '15s', target: 10000 },
        { duration: '30s', target: 10000 },
        { duration: '10s', target: 0 },
    ],
    thresholds: {
        http_req_failed: ['rate<0.05'],
        http_req_duration: ['p(95)<500'],
    },
};

export default function () {
    const port = __ENV.PORT || 9191;
    const res = http.get(`http://127.0.0.1:${port}/`);
    check(res, {
        'status 200': (r) => r.status === 200,
        'has message': (r) => r.body != null && r.body.indexOf('message') !== -1,
    });
}
