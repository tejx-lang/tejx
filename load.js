import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
    stages: [
        { duration: '30s', target: 1000 },
        { duration: '1m', target: 1000 },
        { duration: '30s', target: 0 },
    ],
    thresholds: {
        http_req_failed: ['rate<0.01'],
        http_req_duration: ['p(95)<500'],
    },
};

export default function () {
    const responses = http.batch([
        ['GET', 'http://127.0.0.1:8080/sfsaf', null, { tags: { name: 'API_One' } }],
        ['GET', 'http://127.0.0.1:8080', null, { tags: { name: 'API_Two' } }],
        ['GET', 'http://127.0.0.1:8080/internal/data', null, { tags: { name: 'API_Three' } }],
        ['GET', 'http://127.0.0.1:8080/composite', null, { tags: { name: 'API_Four' } }],
    ]);

    check(responses[0], { 'API One status is 200': (r) => r.status === 200 });
    check(responses[1], { 'API Two status is 200': (r) => r.status === 200 });
    check(responses[2], { 'API Three status is 200': (r) => r.status === 200 });
    check(responses[3], { 'API Four status is 200': (r) => r.status === 200 });

    sleep(1);
}
