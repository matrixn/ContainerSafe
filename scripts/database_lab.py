"""Opt-in Linux Docker/Restic lab. Creates only UUID-named disposable containers.

No published ports, production connections, or existing containers/volumes are used.
Run: python3 scripts/database_lab.py --restic /absolute/path/to/restic
Images are downloaded by Docker when absent; no image pruning is performed.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--restic', required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    script = (root / 'src/database_dump.sh').read_text()
    backup = (root / 'src/direct_backup.sh').read_text()
    containers = []
    secret = uuid.uuid4().hex

    def run(argv, *, input=None, ok=True, timeout=300):
        result = subprocess.run(argv, input=input, capture_output=True, timeout=timeout)
        if ok and result.returncode:
            # Do not log docker args, SQL, credentials, or database stderr.
            raise AssertionError(f'{Path(argv[0]).name} failed with exit {result.returncode}')
        return result

    def docker(*argv, **kw):
        return run(['docker', *argv], **kw)

    def start(image, *argv, env=(), network="none", entrypoint=None):
        name = 'containersafe-db-lab-' + uuid.uuid4().hex
        options = [option for item in env for option in ('-e', item)]
        if entrypoint:
            options += ['--entrypoint', entrypoint]
        cid = docker('run', '-d', '--name', name, '--network', network, *options, image, *argv).stdout.decode().strip()
        containers.append(cid)
        return cid

    def execute(cid, *argv, **kw):
        return docker('exec', '-i', cid, *argv, **kw)

    def wait(cid, *argv):
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if execute(cid, *argv, ok=False).returncode == 0:
                return
            time.sleep(2)
        raise AssertionError('Isolated database readiness timeout')

    try:
        pg = start('postgres:17-alpine', env=['POSTGRES_PASSWORD=' + secret])
        maria = start('mariadb:11.4', env=['MARIADB_ROOT_PASSWORD=' + secret])
        mysql = start('mysql:8.4', env=['MYSQL_ROOT_PASSWORD=' + secret])
        redis = start('redis:7-alpine', 'sh', '-c', 'umask 077; printf "requirepass %s\\n" "$REDIS_PASSWORD" > /tmp/lab.conf; exec redis-server /tmp/lab.conf', env=['REDIS_PASSWORD=' + secret])
        mongo = start('mongo:8', '--replSet', 'lab', '--bind_ip', '127.0.0.1', entrypoint='mongod', env=['MONGO_INITDB_ROOT_USERNAME=lab', 'CONTAINERSAFE_MONGO_CONFIG=/tmp/containersafe-mongo.yml'])
        sqlite = start('alpine:3.22', 'sleep', 'infinity', network='bridge')
        execute(sqlite, 'sh', '-c', 'apk add --no-cache sqlite >/dev/null')
        docker('network', 'disconnect', 'bridge', sqlite)
        wait(pg, 'pg_isready', '-U', 'postgres')
        wait(maria, 'sh', '-c', 'MYSQL_PWD="$MARIADB_ROOT_PASSWORD" mariadb --host=127.0.0.1 -uroot -N -e "SELECT 1"')
        wait(mysql, 'sh', '-c', 'MYSQL_PWD="$MYSQL_ROOT_PASSWORD" mysql --host=127.0.0.1 -uroot -N -e "SELECT 1"')
        wait(redis, 'sh', '-c', 'REDISCLI_AUTH="$REDIS_PASSWORD" redis-cli ping')
        wait(mongo, 'mongosh', '--quiet', '--eval', 'db.adminCommand({ping:1})')
        execute(mongo, 'mongosh', '--quiet', '--eval', 'rs.initiate({_id:"lab",members:[{_id:0,host:"127.0.0.1:27017"}]})')
        wait(mongo, 'mongosh', '--quiet', '--eval', 'if(!db.hello().isWritablePrimary) quit(1)')
        execute(mongo, 'mongosh', '--quiet', input=('db.getSiblingDB("admin").createUser({user:"lab",pwd:' + json.dumps(secret) + ',roles:["root"]})').encode())
        execute(mongo, 'sh', '-c', 'umask 077; cat > /tmp/containersafe-mongo.yml', input=('password: ' + json.dumps(secret) + '\n').encode())
        execute(pg, 'psql', '-U', 'postgres', '-v', 'ON_ERROR_STOP=1', '-c', 'CREATE TABLE fixture(value text); INSERT INTO fixture VALUES (\'retained\');')
        sql = b"CREATE DATABASE fixture; USE fixture; CREATE TABLE sample(value varchar(40)); INSERT INTO sample VALUES ('retained');"
        for cid, client, var in [(maria, 'mariadb', 'MARIADB_ROOT_PASSWORD'), (mysql, 'mysql', 'MYSQL_ROOT_PASSWORD')]:
            execute(cid, 'sh', '-c', f'MYSQL_PWD="${var}" {client} -uroot', input=sql)
        execute(redis, 'sh', '-c', 'REDISCLI_AUTH="$REDIS_PASSWORD" redis-cli set fixture retained')
        execute(mongo, 'mongosh', '--quiet', '--eval', 'db.getSiblingDB("fixture").sample.insertOne({value:"retained"})')
        execute(sqlite, 'sqlite3', '/tmp/live.sqlite', "CREATE TABLE sample(value TEXT); INSERT INTO sample VALUES('retained');")
        specs = [(pg, 'PostgreSQL', ''), (maria, 'MariaDB', ''), (mysql, 'MySQL', ''), (redis, 'Redis/Valkey', ''), (mongo, 'MongoDB', ''), (sqlite, 'SQLite', '/tmp/live.sqlite')]
        with tempfile.TemporaryDirectory(prefix='containersafe-db-lab-') as temp:
            base = Path(temp)
            restores = base / 'restores'
            restores.mkdir(mode=0o700)
            stage = restores / ('containersafe-db-' + str(uuid.uuid4()))
            repository = base / 'repository'
            repository.mkdir()
            password = base / 'restic.password'
            password.write_text(secret)
            password.chmod(0o600)
            common = [str(args.restic.resolve()), str(password), str(repository), str(restores)]
            run(['sh', '-s', '--', 'init', *common, '', ''], input=backup.encode())
            arguments = [str(stage), 'docker', str(repository), str(password), str(len(specs))]
            for index, (cid, engine, path) in enumerate(specs):
                arguments += [cid, engine, path, f'database-{index}']
            arguments += [str(stage)]
            prepared = run(['sh', '-s', '--', 'prepare', *arguments], input=script.encode())
            assert not prepared.stdout and secret.encode() not in prepared.stderr
            assert stage.stat().st_mode & 0o777 == 0o700
            manifest = json.loads((stage / 'manifest.json').read_text())
            assert len(manifest['files']) == 6
            original = {cid: (stage / (cid + '.dump')).read_bytes() for cid, _, _ in specs}
            assert all(original.values())
            # Reusing a stage or overlapping source must fail without deleting a stage.
            assert run(['sh', '-s', '--', 'prepare', *arguments], input=script.encode(), ok=False).returncode == 81
            result = run(['sh', '-s', '--', 'backup', *common, '', '', str(stage)], input=backup.encode())
            summary = [json.loads(line) for line in result.stdout.splitlines() if json.loads(line).get('message_type') == 'summary'][0]
            snapshot = summary['snapshot_id']
            run(['sh', '-s', '--', 'cleanup', *arguments], input=script.encode())
            assert not stage.exists()
            # Reject insecure stage parents and source/repository overlap before exporting.
            restores.chmod(0o777)
            assert run(['sh', '-s', '--', 'prepare', *arguments], input=script.encode(), ok=False).returncode == 80
            restores.chmod(0o700)
            overlapping = arguments[:-1] + [str(restores)]
            assert run(['sh', '-s', '--', 'prepare', *overlapping], input=script.encode(), ok=False).returncode == 80
            assert not stage.exists()
            run(['sh', '-s', '--', 'check', *common, '', '', str(stage)], input=backup.encode())
            run(['sh', '-s', '--', 'restore', *common, snapshot, 'isolated', str(stage)], input=backup.encode())
            restored = restores / 'isolated' / stage.relative_to('/')
            for cid, _, _ in specs:
                assert (restored / (cid + '.dump')).read_bytes() == original[cid]
            # Import only into new test database names / flushed disposable instances.
            execute(pg, 'psql', '-U', 'postgres', '-v', 'ON_ERROR_STOP=1', '-c', 'DROP TABLE fixture;')
            # pg_dumpall includes existing postgres role; psql import intentionally allows that role error.
            execute(pg, 'psql', '-U', 'postgres', input=original[pg])
            assert b'retained' in execute(pg, 'psql', '-U', 'postgres', '-Atc', 'SELECT value FROM fixture').stdout
            for cid, client, var in [(maria, 'mariadb', 'MARIADB_ROOT_PASSWORD'), (mysql, 'mysql', 'MYSQL_ROOT_PASSWORD')]:
                execute(cid, 'sh', '-c', f'MYSQL_PWD="${var}" {client} -uroot -e "DROP DATABASE fixture"')
                execute(cid, 'sh', '-c', f'MYSQL_PWD="${var}" {client} -uroot', input=original[cid])
                assert b'retained' in execute(cid, 'sh', '-c', f'MYSQL_PWD="${var}" {client} -uroot -N -e "SELECT value FROM fixture.sample"').stdout
            execute(mongo, 'mongosh', '--quiet', '--eval', 'db.getSiblingDB("fixture").dropDatabase()')
            execute(mongo, 'mongorestore', '--archive', '--gzip', '--oplogReplay', '--drop', input=original[mongo])
            assert b'retained' in execute(mongo, 'mongosh', '--quiet', '--eval', 'db.getSiblingDB("fixture").sample.findOne().value').stdout
            execute(sqlite, 'sh', '-c', 'cat > /tmp/restored.sqlite', input=original[sqlite])
            assert b'retained' in execute(sqlite, 'sqlite3', '/tmp/restored.sqlite', 'SELECT value FROM sample;').stdout
            execute(redis, 'sh', '-c', 'cat > /tmp/restored.rdb', input=original[redis])
            execute(redis, 'redis-check-rdb', '/tmp/restored.rdb')
            execute(redis, 'redis-server', '--port', '6380', '--bind', '127.0.0.1', '--dir', '/tmp', '--dbfilename', 'restored.rdb', '--appendonly', 'no', '--save', '', '--daemonize', 'yes')
            assert b'retained' in execute(redis, 'redis-cli', '-p', '6380', 'get', 'fixture').stdout
            execute(redis, 'redis-cli', '-p', '6380', 'shutdown', 'nosave')
            # Missing tool/unsupported topology fails; no stale stage is left behind.
            bad = arguments[:4] + ['1', sqlite, 'PostgreSQL', '', 'missing-client', str(stage)]
            assert run(['sh', '-s', '--', 'prepare', *bad], input=script.encode(), ok=False).returncode == 85
            assert not stage.exists()
            standalone = start('mongo:8', '--bind_ip', '127.0.0.1')
            wait(standalone, 'mongosh', '--quiet', '--eval', 'db.adminCommand({ping:1})')
            unsupported = arguments[:4] + ['1', standalone, 'MongoDB', '', 'standalone', str(stage)]
            assert run(['sh', '-s', '--', 'prepare', *unsupported], input=script.encode(), ok=False).returncode == 89
            clustered = start('redis:7-alpine', 'redis-server', '--cluster-enabled', 'yes')
            wait(clustered, 'redis-cli', 'ping')
            unsupported = arguments[:4] + ['1', clustered, 'Redis/Valkey', '', 'clustered', str(stage)]
            assert run(['sh', '-s', '--', 'prepare', *unsupported], input=script.encode(), ok=False).returncode == 88
            assert not stage.exists()
            print(json.dumps({'status': 'PASS', 'engines': [v[1] for v in specs], 'encrypted_snapshot': True, 'verified_restore': True, 'production_connections': 0}))
    finally:
        for cid in reversed(containers):
            docker('rm', '-f', '-v', cid, ok=False)


if __name__ == '__main__':
    main()
