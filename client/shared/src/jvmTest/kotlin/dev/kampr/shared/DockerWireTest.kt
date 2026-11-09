package dev.kampr.shared

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.DockerAction
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.dockerDetailOf
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val HERD = """
{"t":"herd",
 "nodes":[
  {"id":"01JHOST","name":"front","kind":"local","online":true,"docker":true,
   "containers":[
    {"id":"f49c60fac115","name":"kobbler-postgres-test-1","image":"postgres:18.6","state":"running",
     "health":"healthy","started_at":1794219336,"restarts":3,"crashes":4,
     "last_crash":{"at":1794219337,"exit_code":137,"oom":true},
     "project":"kobbler","service":"postgres-test","ports":["5432→5432/tcp"],"child":"01JHOST-c-f49c60fac115"},
    {"id":"bad","name":"broken","restarts":"many"},
    {"id":"e1","name":"old","image":"busybox","state":"exited","finished_at":1794212000,"exit_code":137,"oom":true,"future":{"x":1}}
   ]},
  {"id":"01JHOST-c-f49c60fac115","name":"kobbler-postgres-test-1","kind":"peer","online":false,"parent":"01JHOST"},
  {"id":"01JODD","name":"odd","containers":"not a list"}
 ],
 "panes":[{"id":"01JHOST/n:s1","node_id":"01JHOST","provider":"native","container":"kobbler-postgres-test-1"}]}
"""

class DockerWireTest {
    private fun herd() = Wire.decode(HERD) as ServerMsg.Herd

    @Test
    fun aNodeCarriesItsContainersAndAMalformedOneCostsOnlyItself() {
        val herd = herd()
        assertEquals(listOf("01JHOST", "01JHOST-c-f49c60fac115", "01JODD"), herd.nodes.map { it.id })
        val host = herd.nodes.first()
        assertEquals(true, host.docker)
        assertEquals(listOf("kobbler-postgres-test-1", "old"), host.containers.map { it.name })
        val pg = host.containers.first()
        assertEquals(1794219336L, pg.startedAt)
        assertEquals(3, pg.restarts)
        assertEquals(4, pg.crashes)
        assertEquals(137, pg.lastCrash?.exitCode)
        assertEquals(true, pg.lastCrash?.oom)
        assertEquals("kobbler", pg.project)
        assertEquals(listOf("5432→5432/tcp"), pg.ports)
        assertEquals("01JHOST-c-f49c60fac115", pg.child)
        val old = host.containers[1]
        assertEquals(137, old.exitCode)
        assertTrue(old.oom)
        assertNull(old.startedAt)
    }

    @Test
    fun anAdoptedNodeNamesItsParentAndAShellNamesItsContainer() {
        val herd = herd()
        assertEquals("01JHOST", herd.nodes[1].parent)
        assertNull(herd.nodes[0].parent)
        assertNull(herd.nodes[2].docker)
        assertTrue(herd.nodes[2].containers.isEmpty())
        assertEquals("kobbler-postgres-test-1", herd.panes.single().container)
    }

    @Test
    fun theDetailRidesOnTheManagedAck() {
        val ack = Wire.decode(
            """{"t":"managed","op":"docker.detail","ok":true,"detail":{
                 "id":"f49c60fac115","name":"kobbler-postgres-test-1","state":"running","started_at":1794219336,
                 "command":["docker-entrypoint.sh","postgres"],"restart_policy":"unless-stopped",
                 "limits":{"memory":1073741824},"env":["POSTGRES_DB=kobbler",7],"labels":{"a":"b"},
                 "mounts":[{"src":"/srv/pg","dst":"/var/lib/postgresql","mode":"rw"}],
                 "health_log":[{"start":"2026-11-09T10:00:00Z","exit_code":1,"output":"pg_isready: no response"}],
                 "logs":[{"at":1794219340,"stream":"stderr","line":"FATAL: the database system is starting up"}],
                 "stats":{"cpu_pct":12.5,"mem_used":44040192,"mem_limit":1073741824,"pids":9},
                 "events":[{"at":1794219337,"action":"oom"},{"at":1794219337,"action":"die","exit_code":137}]}}""",
        ) as ServerMsg.Managed
        val detail = assertNotNull(dockerDetailOf(ack.detail))
        assertEquals("kobbler-postgres-test-1", detail.entry.name)
        assertEquals(12.5, detail.more.stats?.cpuPct)
        assertEquals(44040192L, detail.more.stats?.memUsed)
        assertEquals(listOf("POSTGRES_DB=kobbler", "7"), detail.more.env)
        assertEquals("/var/lib/postgresql", detail.more.mounts.single().dst)
        assertEquals("pg_isready: no response", detail.more.healthLog.single().output)
        assertEquals("FATAL: the database system is starting up", detail.more.logs.single().line)
        assertEquals(listOf("oom", "die"), detail.more.events.map { it.action })
        assertEquals(137, detail.more.events[1].exitCode)
    }

    @Test
    fun aFailingAdoptionSaysWhyAndAGoneContainerIsStillListed() {
        val herd = Wire.decode(
            """{"t":"herd","nodes":[{"id":"01JHOST","name":"front","containers":[
                 {"id":"c1","name":"db","image":"postgres","state":"running","child":"01JHOST-c-c1",
                  "adopt_error":"db has a read-only root filesystem"},
                 {"id":"gone","name":"gone","image":"","state":"missing","child":"01JHOST-c-aa",
                  "adopt_error":"gone no longer exists"},
                 {"id":"c3","name":"fine","state":"running"}]}],"panes":[]}""",
        ) as ServerMsg.Herd
        val containers = herd.nodes.single().containers
        assertEquals(
            listOf("db has a read-only root filesystem", "gone no longer exists", null),
            containers.map { it.adoptError },
        )
        assertEquals("missing", containers[1].state)
    }

    @Test
    fun everyDockerOpNamesTheNodeAndTheContainer() {
        val expected = mapOf(
            DockerAction.Detail to "docker.detail",
            DockerAction.Shell to "docker.shell",
            DockerAction.Start to "docker.start",
            DockerAction.Stop to "docker.stop",
            DockerAction.Restart to "docker.restart",
            DockerAction.Kill to "docker.kill",
            DockerAction.Remove to "docker.remove",
            DockerAction.Adopt to "docker.adopt",
            DockerAction.Release to "docker.release",
        )
        assertEquals(DockerAction.entries.toSet(), expected.keys)
        for ((action, op) in expected) {
            assertEquals(
                """{"t":"manage","op":"$op","node":"01JHOST","id":"f49c60fac115"}""",
                Wire.encode(ClientMsg.Manage(ManageOp.Docker(action, "01JHOST", "f49c60fac115"))),
            )
        }
    }
}
