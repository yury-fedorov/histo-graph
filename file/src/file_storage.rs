use histo_graph_core::graph::{
    graph::{VertexId, Edge},
    directed_graph::DirectedGraph,
};

use crate::error::{Error, Result};

use ring::digest::{Context, SHA256};
use data_encoding::HEXLOWER;
use serde::{Serialize, Deserialize};

use futures::future::Future;
use std::{
    borrow::Borrow,
    io,
    path::{Path, PathBuf},
};
use std::ffi::OsStr;


#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    fn to_string(&self) -> String {
        HEXLOWER.encode(&self.0)
    }
}

impl<T> From<T> for Hash
    where T: AsRef<[u8]> {
    fn from(content: T) -> Hash {
        let mut context = Context::new(&SHA256);
        context.update(content.as_ref());
        let digest = context.finish();
        let mut hash: [u8; 32] = [0u8; 32];
        hash.copy_from_slice(digest.as_ref());

        Hash(hash)
    }
}

struct File {
    content: Vec<u8>,
    hash: Hash,
}

/// A HashEdge respresents an edge by the hashes of the vertices it is connected to.
#[derive(Serialize, Deserialize)]
struct HashEdge {
    from: Hash,
    to: Hash,
}

/// The root of a stored graph. It holds the hashes of the vertex vector and the edge vector.
#[derive(Serialize, Deserialize)]
pub struct GraphHash {
    vertex_vec_hash: Hash,
    edge_vec_hash: Hash,
}

fn vertex_to_file(vertex_id: &VertexId) -> File {
    // serialize the vertex_id
    let content: Vec<u8> = bincode::serialize(&vertex_id.0).unwrap();
    let hash: Hash = (&content).into();

    File {
        content,
        hash,
    }
}

fn edge_to_file(edge: &Edge) -> File {
    let File { hash: v_hash_0, ..} = vertex_to_file(&edge.0);
    let File { hash: v_hash_1, ..} = vertex_to_file(&edge.1);

    let hash_edge = HashEdge { from: v_hash_0,  to: v_hash_1};

    let content: Vec<u8> = bincode::serialize(&hash_edge).unwrap();
    let hash: Hash = (&content).into();

    File {
        content,
        hash,
    }
}

fn hash_vec_to_file(hash_vec: &Vec<Hash>) -> File {
    // serialize the vertex_id
    let content: Vec<u8> = bincode::serialize(&hash_vec).unwrap();
    let hash: Hash = (&content).into();

    File {
        content,
        hash,
    }
}

fn file_to_vertex(file: &File) -> Result<VertexId> {
    let id: u64 = bincode::deserialize(file.content.as_ref())?;
    Ok(VertexId(id))
}

fn file_to_hash_edge(file: &File) -> Result<HashEdge> {
    bincode::deserialize(file.content.as_ref())
        .map_err(Into::into)
}

fn file_to_hash_vec(file: &File) -> Result<Vec<Hash>> {
    let result = bincode::deserialize(file.content.as_ref())?;
    Ok(result)
}

fn write_file_in_dir(dir_path: &Path, file: File) -> impl Future<Error = io::Error> {
    let path = dir_path.join(&file.hash.to_string());
    tokio_fs::write(path, file.content)
}

/// Writes vertices to files.
///
/// First creates a sub-directory `vertex/` in the provided `base_path`, then writes the vertices
/// into this sub-directory, creating one file for each vertex.
/// Returns a vector of the hashes of the written files.
fn write_all_vertices_to_files<I>(base_path: PathBuf, i: I) -> impl Future<Item=Vec<Hash>, Error = io::Error>
    where I: IntoIterator,
          <I as IntoIterator>::Item: Borrow<VertexId>
{
    let path = base_path.join("vertex");
    let futs = i
        .into_iter()
        .map(| v | vertex_to_file(v.borrow()))
        .map({
            let path = path.clone();
            move |f| {
                let hash = f.hash;
                write_file_in_dir(path.as_ref(), f)
                    .map(move |_| hash)
            }
        });

    tokio_fs::create_dir_all(path)
        .and_then(| _ | futures::future::join_all(futs))
}

/// Writes the vector of hashes of the vertices of a graph to a file.
///
/// First creates a sub-directoy `vertexvec/` in the provided `base_path`, then writes the vector
/// of hashes into a single file in that sub-directory.
/// Returns a hash of the written file.
fn write_vertex_hash_vec_file(base_path: PathBuf, hash_vec: Vec<Hash>) -> impl Future<Item = Hash, Error = io::Error> {
    let path = base_path.join("vertexvec");
    let file = hash_vec_to_file(&hash_vec);
    let hash = file.hash;

    tokio_fs::create_dir_all(path.clone())
        .and_then(move | _ | write_file_in_dir(&path, file))
        .map( move | _ | hash)
}

/// Writes the vertices of a graph.
/// Returns the hash of the vertex vector file.
fn write_graph_vertices(base_path: PathBuf, graph: &DirectedGraph) -> impl Future<Item = Hash, Error = io::Error> {
    let vertices: Vec<VertexId> = graph
        .vertices()
        .map(| v | *v)
        .collect();

    tokio_fs::create_dir_all(base_path.clone())
        .and_then({ let base_path = base_path.clone(); move | _ | {
            write_all_vertices_to_files(base_path, vertices)
        }})
        .and_then(move | hash_vec |
            write_vertex_hash_vec_file(base_path, hash_vec)
        )
}

/// Writes an edge to a file in the directory specified by `dir_path`.
/// Returns the hash of the file.
#[cfg(test)]
fn write_edge_to_file(dir_path: PathBuf, edge: &Edge) -> impl Future<Item = Hash, Error = io::Error> {
    let file = edge_to_file(edge);
    let hash = file.hash;
    write_file_in_dir(&dir_path, file)
        .map(move | _ | hash)
}

/// Writes edges to files.
///
/// First creates a sub-directory `edge/` in the provided `base_path`, then writes the edges
/// into this sub-directory, creating one file for each edge.
/// Returns a vector of the hashes of the written files.
fn write_all_edges_to_files<I>(base_path: PathBuf, i: I) -> impl Future<Item=Vec<Hash>, Error = io::Error>
    where I: IntoIterator,
          <I as IntoIterator>::Item: Borrow<Edge>
{
    let path = base_path.join("edge");
    let futs = i
        .into_iter()
        .map(| e | edge_to_file(e.borrow()))
        .map({
            let path = path.clone();
            move |f| {
                let hash = f.hash;
                write_file_in_dir(path.as_ref(), f)
                    .map(move |_| hash)
            }
        });

    tokio_fs::create_dir_all(path)
        .and_then(| _ | futures::future::join_all(futs))
}

/// Writes the vector of hashes of the edges of a graph to a file.
///
/// First creates a sub-directoy `edgevec/` in the provided `base_path`, then writes the vector
/// of hashes into a single file in that sub-directory.
/// Returns a hash of the written file.
fn write_edge_hash_vec_file(base_path: PathBuf, hash_vec: Vec<Hash>) -> impl Future<Item = Hash, Error = io::Error> {
    let path = base_path.join("edgevec");
    let file = hash_vec_to_file(&hash_vec);
    let hash = file.hash;

    tokio_fs::create_dir_all(path.clone())
        .and_then(move | _ | write_file_in_dir(&path, file))
        .map( move | _ | hash)
}

/// Writes the edges of a graph.
/// Returns the hash of the edge vector file.
fn write_graph_edges(base_path: PathBuf, graph: &DirectedGraph) -> impl Future<Item = Hash, Error = io::Error> {
    let edges: Vec<Edge> = graph
        .edges()
        .map(| v | *v)
        .collect();

    tokio_fs::create_dir_all(base_path.clone())
        .and_then({ let base_path = base_path.clone(); move | _ | {
            write_all_edges_to_files(base_path, edges)
        }})
        .and_then(move | hash_vec |
            write_edge_hash_vec_file(base_path, hash_vec)
        )
}

/// Writes the vertices and edges of a graph.
/// Returns a `GraphHash`.
pub fn write_graph(base_path: PathBuf, graph: &DirectedGraph) -> impl Future<Item = GraphHash, Error = io::Error> {
    let vertex_fut = write_graph_vertices(base_path.clone(), graph);
    let edge_fut = write_graph_edges(base_path, graph);

    vertex_fut.join(edge_fut)
        .map(|(vertex_vec_hash, edge_vec_hash)| GraphHash{vertex_vec_hash, edge_vec_hash})
}

/// Saves a graph under the given name.
///
/// Creates a subdirectory `graph/` of the provided base_path, then saves the serialized GraphHash
/// of the provided graph in that directory.
/// Returns the path to the written file.
pub fn save_graph_as(base_path: PathBuf, name: &OsStr, graph: &DirectedGraph) -> impl Future<Item=PathBuf, Error=Error> {
    let dir = base_path.join("graph");
    let path = dir.join(name);
    write_graph(base_path, graph)
        .map_err(Into::<Error>::into)
        .and_then(move |graph_hash| tokio_fs::create_dir_all(dir)
            .map_err(Into::<Error>::into)
            .and_then(move | _ | bincode::serialize(&graph_hash)
                .map_err(Into::<Error>::into))
        )
        .and_then({
            let path = path.clone();
            move |content| tokio_fs::write(path, content)
                .map_err(Into::<Error>::into)
        })
        .map(|_| path)
}


fn read_file_in_dir(dir_path: &Path, hash: Hash) -> impl Future<Item = File, Error = io::Error> {
    let path = dir_path.join(hash.to_string());
    tokio_fs::read(path)
        .map( move |content| File {
            content,
            hash
        })
}

/// Reads a vertex hash vector file.
///
/// Reads from a file placed in the sub-directory `vertexvec/` of the provided base_path, with the
/// provided `hash` as a filename.
/// Returns a hash vector.
fn read_vertex_hash_vec(base_path: PathBuf, hash: Hash) -> impl Future<Item = Vec<Hash>, Error = Error> {
    let path = base_path
        .join("vertexvec");

    read_file_in_dir(&path, hash)
        .map_err(Into::into)
        .and_then(|file| file_to_hash_vec(&file) )
}

/// Reads vertices from files.
///
/// Reads from files placed in the sub-directory `vertex/` of the provided base_path.
/// Where the filenames are given by the provided hash_vec.
fn read_all_vertices_from_files(base_path: PathBuf, hash_vec: Vec<Hash>) -> impl Future<Item = Vec<VertexId>, Error = Error> {
    let path = base_path.join("vertex");

    let futs = hash_vec
        .into_iter()
        .map(move |hash| {
            read_file_in_dir(&path, hash)
                .map_err(Into::into)
                .and_then(|file| file_to_vertex(&file))
        });

    futures::future::join_all(futs)
}

/// Reads vertices and adds them to the provided graph.
///
/// Note that this function consumes the graph, and returns it back in the returned Future, with
/// the vertices added.
fn read_graph_vertices(base_path: PathBuf, hash: Hash, mut graph: DirectedGraph) -> impl Future<Item=DirectedGraph, Error=Error> {
    read_vertex_hash_vec(base_path.clone(), hash)
        .and_then(move |hash_vec| read_all_vertices_from_files(base_path, hash_vec))
        .and_then(|vertices| {
            for v in vertices {
                graph.add_vertex(v);
            }
            Ok(graph)
        })
}

fn read_hash_edge(dir_path: PathBuf, hash: Hash) -> impl Future<Item = HashEdge, Error = Error> {
    read_file_in_dir(&dir_path, hash)
        .map_err(Into::into)
        .and_then(|file| file_to_hash_edge(&file))
}

fn read_edge(base_path: &PathBuf, hash: Hash) -> impl Future<Item = Edge, Error = Error> {
    let edge_path = base_path.join("edge");
    let vertex_path = base_path.join("vertex");

    read_hash_edge(edge_path, hash)
        .and_then(move |HashEdge { from, to}| {
            let from_fut = read_file_in_dir(&vertex_path, from)
                .map_err(Into::into)
                .and_then(|file| file_to_vertex(&file));
            let to_fut = read_file_in_dir(&vertex_path, to)
                .map_err(Into::into)
                .and_then(|file| file_to_vertex(&file));

            from_fut.join(to_fut)
                .map(|(v0, v1)| Edge(v0, v1))
        })
}

/// Reads an edge hash vector file.
///
/// Reads from a file placed in the sub-directory `edgevec/` of the provided base_path, with the
/// provided `hash` as a filename.
/// Returns a hash vector.
fn read_edge_hash_vec(base_path: PathBuf, hash: Hash) -> impl Future<Item = Vec<Hash>, Error = Error> {
    let path = base_path
        .join("edgevec");

    read_file_in_dir(&path, hash)
        .map_err(Into::into)
        .and_then(|file| file_to_hash_vec(&file) )
}

/// Reads edge from files.
///
/// Reads edge from files placed in the sub-directory `edge/` of the provided base_path.
/// Where the filenames are given by the provided hash_vec.
/// Also reades the vertices connected to the edges from a subdirectory `vertex/` of the provided
/// base_path.
fn read_all_edges_from_files(base_path: PathBuf, hash_vec: Vec<Hash>) -> impl Future<Item = Vec<Edge>, Error = Error> {
    let futs = hash_vec
        .into_iter()
        .map(move |hash| read_edge(&base_path, hash));

    futures::future::join_all(futs)
}

/// Reads edges and adds them to the provided graph.
///
/// Note that this function consumes the graph, and returns it back in the returned Future, with
/// the edges added.
fn read_graph_edges(base_path: PathBuf, hash: Hash, mut graph: DirectedGraph) -> impl Future<Item=DirectedGraph, Error=Error> {
    read_edge_hash_vec(base_path.clone(), hash)
        .and_then(move |hash_vec| read_all_edges_from_files(base_path, hash_vec))
        .and_then(|edges| {
            for e in edges {
                graph.add_edge(e);
            }
            Ok(graph)
        })
}

/// Reads the vertices and edges of a graph, specified by the provided graph_hash.
pub fn read_graph(base_path: PathBuf, graph_hash: GraphHash) -> impl Future<Item = DirectedGraph, Error = Error> {
    let graph = DirectedGraph::new();

    read_graph_vertices(base_path.clone(), graph_hash.vertex_vec_hash, graph)
        .and_then(move |graph| read_graph_edges(base_path, graph_hash.edge_vec_hash, graph))
}

pub fn load_graph(base_dir: PathBuf, name: &OsStr) -> impl Future<Item = DirectedGraph, Error = Error> {
    let path = base_dir.join("graph").join(name);
    tokio_fs::read(path)
        .map_err(Into::<Error>::into)
        .and_then(|content| bincode::deserialize::<GraphHash>(&content)
            .map_err(Into::<Error>::into))
        .and_then(|graph_hash| read_graph(base_dir, graph_hash))
}

#[cfg(test)]
mod test {
    use histo_graph_core::graph::graph::VertexId;
    use super::*;
    use futures::future::Future;
    use tokio::runtime::Runtime;
    use std::path::{Path, PathBuf};
    use std::ffi::OsString;
    use histo_graph_core::graph::directed_graph::DirectedGraph;
    use crate::error::Result;

    #[test]
    fn test_hash() {
        let File{content: _, hash} = vertex_to_file(&VertexId(27));

        assert_eq!(hash.to_string(), "4d159113222bfeb85fbe717cc2393ee8a6a85b7ce5ac1791c4eade5e3dd6de41")
    }

    #[test]
    fn test_write_and_read_vertex() -> Result<()> {
        let vertex = VertexId(18);

        let file = vertex_to_file(&vertex);
        let hash = file.hash;

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = write_file_in_dir(&path, file)
            .and_then(move | _ | read_file_in_dir(&path, hash));

        let mut rt = Runtime::new()?;
        let file = rt.block_on(f)?;

        let result = file_to_vertex(&file)?;

        assert_eq!(result, vertex);

        Ok(())
    }

    #[test]
    fn test_write_vertices() -> Result<()> {
        let vertices = vec!{VertexId(1), VertexId(2), VertexId(3), VertexId(4)};

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = write_all_vertices_to_files(path, vertices.into_iter());

        let mut rt = Runtime::new()?;
        rt.block_on(f)?;

        Ok(())
    }

    #[test]
    fn test_write_and_read_graph_vertices() -> Result<()> {
        let mut graph = DirectedGraph::new();
        graph.add_vertex(VertexId(27));
        graph.add_vertex(VertexId(28));
        graph.add_vertex(VertexId(29));

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = write_graph_vertices(path.clone(), &graph)
            .map_err(Into::into)
            .and_then(|hash|{
                let result_graph = DirectedGraph::new();
                read_graph_vertices(path, hash, result_graph)
            });

        let mut rt = Runtime::new()?;
        let result_graph = rt.block_on(f)?;

        assert_eq!(graph, result_graph);

        Ok(())
    }

    #[test]
    fn test_write_and_read_edge() -> Result<()> {
        let vertices = vec![VertexId(42), VertexId(43)];
        let edge = Edge(VertexId(42), VertexId(43));

        let path: PathBuf = Path::new("../target/test/store/").into();
        let edge_path: PathBuf = path.join("edge");

        let f = write_all_vertices_to_files(path.clone(), vertices)
            .and_then({ let edge_path = edge_path.clone(); move | _ | tokio_fs::create_dir_all(edge_path)})
            .and_then(move | _ | write_edge_to_file(edge_path, &edge))
            .map_err(Into::into)
            .and_then(move |hash| read_edge(&path, hash));

        let mut rt = Runtime::new()?;
        let result_edge = rt.block_on(f)?;

        assert_eq!(edge, result_edge);

        Ok(())
    }

    #[test]
    fn test_write_graph_edges() -> Result<()> {
        let mut graph = DirectedGraph::new();
        graph.add_edge(Edge(VertexId(3), VertexId(4)));
        graph.add_edge(Edge(VertexId(3), VertexId(5)));
        graph.add_edge(Edge(VertexId(4), VertexId(5)));

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = write_graph_vertices(path.clone(), &graph)
            .and_then(move | _ | write_graph_edges(path, &graph))
            .map(| _ | ());


        let mut rt = Runtime::new()?;
        rt.block_on(f)
            .map_err(Into::into)
    }

    #[test]
    fn test_write_and_read_graph() -> Result<()> {
        let mut graph = DirectedGraph::new();
        graph.add_vertex(VertexId(27));
        graph.add_edge(Edge(VertexId(28), VertexId(29)));
        graph.add_edge(Edge(VertexId(28), VertexId(30)));

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = write_graph(path.clone(), &graph)
            .map_err(Into::into)
            .and_then(|graph_hash| read_graph(path, graph_hash));

        let mut rt = Runtime::new()?;
        let result_graph = rt.block_on(f)?;

        assert_eq!(graph, result_graph);

        Ok(())
    }

    #[test]
    fn test_read_and_write_named_graph() -> Result<()> {
        let mut graph = DirectedGraph::new();
        graph.add_vertex(VertexId(27));
        graph.add_edge(Edge(VertexId(28), VertexId(29)));
        graph.add_edge(Edge(VertexId(28), VertexId(30)));

        let path: PathBuf = Path::new("../target/test/store/").into();

        let f = save_graph_as(path.clone(), &OsString::from("laurengraph"), &graph)
            .and_then(move | _ | load_graph(path, &OsString::from("laurengraph")));

        let mut rt = Runtime::new()?;
        let result_graph = rt.block_on(f)?;

        assert_eq!(graph, result_graph);

        Ok(())
    }
}
